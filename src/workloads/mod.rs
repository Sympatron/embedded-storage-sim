pub mod sequential_storage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    QueuePush,
    QueuePop,
    MapStore,
    MapRemove,
    #[allow(dead_code)]
    MapFetch,
}
impl ToString for Operation {
    fn to_string(&self) -> String {
        match self {
            Operation::QueuePush => "Push".to_string(),
            Operation::QueuePop => "Pop".to_string(),
            Operation::MapStore => "Store".to_string(),
            Operation::MapRemove => "Remove".to_string(),
            Operation::MapFetch => "Fetch".to_string(),
        }
    }
}
