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
            finish_message: finish_message.to_owned(),
        }
    }
    pub fn set_message(&mut self, message: &str) {
        message.clone_into(&mut self.current_message);
        self.display();
    }
    pub fn inc(&mut self) {
        self.current = self.current.saturating_add(1);
        self.display();
    }
    pub fn set_message_and_inc(&mut self, message: &str) {
        message.clone_into(&mut self.current_message);
        self.inc();
    }
    pub fn dec_total(&mut self) {
        self.total = self.total.saturating_sub(1);
        self.display();
    }
    pub fn set_message_and_dec_total(&mut self, message: &str) {
        message.clone_into(&mut self.current_message);
        self.dec_total();
    }
    pub fn display(&self) {
        let progress: u8 = self
            .current
            .saturating_mul(100)
            .checked_div(self.total)
            .unwrap_or(0)
            .min(100)
            .try_into()
            .unwrap_or(100);
        let elapsed = self.start_time.elapsed().as_secs();
        let eta = elapsed
            .saturating_mul(self.total.saturating_sub(self.current))
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
