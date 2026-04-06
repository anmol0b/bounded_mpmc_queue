use std::hint;
use std::thread;

pub struct Backoff {
    step: u32,
}

impl Backoff {
    pub fn new() -> Self {
        Backoff { step: 0 }
    }

    pub fn spin(&mut self) {
        if self.step < 6 {
            for _ in 0..1 << self.step {
                hint::spin_loop();
            }
            self.step += 1;
        } else {
            self.step = 0;
            thread::yield_now();
        }
    }
}