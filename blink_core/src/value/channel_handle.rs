use std::collections::VecDeque;

use crate::ValueRef;

// Add these to your HandleRegistry
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelHandle {
    pub id: u64,
}

#[derive(Debug)]
pub struct ChannelEntry {
    pub generation: u32,
    pub buffer: VecDeque<ValueRef>,
    pub capacity: Option<usize>, // None = unbuffered
    pub waiting_senders: VecDeque<(u32, ValueRef)>, // (goroutine_id, value)
    pub waiting_receivers: VecDeque<u32>, // goroutine_id
    pub closed: bool,
}