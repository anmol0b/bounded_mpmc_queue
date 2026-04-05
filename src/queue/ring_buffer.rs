pub struct RingBuffer<T>{
    buffer: Vec<Option<T>>,
    head: usize,
    tail: usize,
    len: usize,
    capacity: usize,
}
impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> RingBuffer<T>{
        RingBuffer {
            head: 0,
            tail: 0,
            len:  0,
            capacity: capacity,
            buffer: (0..capacity).map(|_| None).collect(),
        }
    }
    pub fn is_empty(&self) -> bool{
        self.len == 0
    }
    pub fn is_full(&self) -> bool{
        self.len == self.capacity    
    }
    pub fn pop(&mut self) -> Option<T>{
        let item = self.buffer[self.head].take();
        self.head = (self.head + 1) % self.capacity;
        self.len -= 1;
        item
    }
    pub fn push(&mut self, item: T){
        self.buffer[self.tail] = Some(item);
        self.tail = (self.tail + 1) % self.capacity;
        self.len += 1;
    }
}