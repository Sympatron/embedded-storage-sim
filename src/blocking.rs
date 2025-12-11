use std::convert::Infallible;

use crate::{SimulatedNorFlash, Transaction};

use embedded_storage::nor_flash::{ErrorType, NorFlash, ReadNorFlash};
use rand::Rng as _;

impl<O, const RS: usize, const WS: usize, const ES: usize> ErrorType
    for SimulatedNorFlash<O, RS, WS, ES>
{
    type Error = Infallible;
}

impl<O: Clone, const RS: usize, const WS: usize, const ES: usize> ReadNorFlash
    for SimulatedNorFlash<O, RS, WS, ES>
{
    const READ_SIZE: usize = RS;

    fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        assert_eq!(0, offset % Self::READ_SIZE as u32);
        assert_eq!(0, bytes.len() % Self::READ_SIZE);
        assert!(offset as usize + bytes.len() <= self.data.len());

        bytes.copy_from_slice(&self.data[offset as usize..offset as usize + bytes.len()]);
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte |= self.stuck_at_1_bits[offset as usize + i];
            *byte &= !self.stuck_at_0_bits[offset as usize + i];
        }

        self.transactions.push(Transaction::read(
            self.log_level,
            offset,
            bytes.len(),
            bytes,
            self.current_operation.clone(),
        ));

        self.read += bytes.len();
        self.read_accesses += 1;
        Ok(())
    }

    fn capacity(&self) -> usize {
        self.data.len()
    }
}
impl<O: Clone, const RS: usize, const WS: usize, const ES: usize> NorFlash
    for SimulatedNorFlash<O, RS, WS, ES>
{
    const WRITE_SIZE: usize = WS;
    const ERASE_SIZE: usize = ES;

    fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        assert_eq!(0, from % Self::ERASE_SIZE as u32);
        assert_eq!(0, to % Self::ERASE_SIZE as u32);
        assert!(from < to);
        assert!((to as usize) <= self.data.len());

        let range = from as usize..to as usize;
        for page in range.clone().step_by(Self::ERASE_SIZE) {
            let page_index = page / Self::ERASE_SIZE;
            self.page_cycles[page_index] += 1;
            if self.page_cycles[page_index] > self.minimum_safe_erase_cycles {
                if (self.page_cycles[page_index] - self.minimum_safe_erase_cycles)
                    % self.bit_failure_every_x_erases
                    == 0
                {
                    // Introduce a stuck-at-1 or stuck-at-0 bit failure at a random location in the page
                    let failure_offset = self.rng.random_range(0..Self::ERASE_SIZE);
                    let global_offset = page + failure_offset;
                    if self.rng.random::<bool>() {
                        // Stuck-at-1
                        self.stuck_at_1_bits[global_offset] |= 1 << self.rng.random_range(0..8);
                    } else {
                        // Stuck-at-0
                        self.stuck_at_0_bits[global_offset] |= 1 << self.rng.random_range(0..8);
                    }
                }
            }
        }
        self.transactions.push(Transaction::erase(
            self.log_level,
            from,
            to,
            &self.data[range.clone()],
            self.current_operation.clone(),
        ));
        self.data[range.clone()].fill(0xff);
        // inject stuck at 0 errors
        for i in range.clone() {
            self.data[i] &= !self.stuck_at_0_bits[i];
        }
        self.erased += (to - from) as usize;
        self.erase_accesses += 1;
        Ok(())
    }

    fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        assert!(offset as usize + bytes.len() <= self.data.len());
        assert_eq!(0, offset % Self::WRITE_SIZE as u32);
        assert_eq!(0, bytes.len() % Self::WRITE_SIZE);

        let range = offset as usize..(offset as usize + bytes.len());
        for (i, byte) in self.data[range.clone()].iter_mut().enumerate() {
            *byte &= bytes[i];
            *byte |= self.stuck_at_1_bits[offset as usize + i];
            *byte &= !self.stuck_at_0_bits[offset as usize + i];
        }
        self.transactions.push(Transaction::write(
            self.log_level,
            offset,
            bytes,
            &self.data[range],
            self.current_operation.clone(),
        ));
        self.written += bytes.len();
        self.write_accesses += 1;
        Ok(())
    }
}
