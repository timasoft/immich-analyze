use std::time::Instant;

/// Simple progress display without external dependencies
pub struct SimpleProgress {
    pub total: u64,
    pub current: u64,
    pub start_time: Instant,
    pub current_message: String,
    pub finish_message: String,
}

impl SimpleProgress {
    pub fn new(total: u64, finish_message: &str) -> Self {
        Self {
            total,
            current: 0,
            start_time: Instant::now(),
            current_message: String::new(),
            finish_message: finish_message.to_string(),
        }
    }
    pub fn set_message(&mut self, message: &str) {
        self.current_message = message.to_string();
        self.display();
    }
    pub fn inc(&mut self) {
        self.current += 1;
        self.display();
    }
    pub fn set_message_and_inc(&mut self, message: &str) {
        self.current_message = message.to_string();
        self.inc();
    }
    pub fn display(&self) {
        let progress = (self.current as f64 / self.total as f64 * 100.0) as u8;
        let elapsed = self.start_time.elapsed().as_secs();
        let eta = (elapsed * (self.total - self.current))
            .checked_div(self.current)
            .unwrap_or(0);
        if progress >= 100 {
            println!(
                "[{:3}%] {}/{} ({}s)",
                progress, self.total, self.total, elapsed
            );
            println!("   {}", self.finish_message);
        } else if self.current_message.is_empty() {
            println!(
                "[{:3}%] {}/{} ({}s, ETA: {}s)",
                progress, self.current, self.total, elapsed, eta
            );
        } else {
            println!(
                "[{:3}%] {}/{} ({}s, ETA: {}s)",
                progress, self.current, self.total, elapsed, eta
            );
            println!("   {}", self.current_message);
        }
    }
}
