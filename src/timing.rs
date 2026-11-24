//! Statistics of a delay measured once per message, shared by the two programs.

/// Running statistics of a delay, in seconds: how many, their sum, how many are long and the worst.
#[derive(Debug, Default, Clone, Copy)]
pub struct Delays {
    pub count: u64,
    pub sum_s: f64,
    pub over_1ms: u64,
    pub worst_s: f64,
}


impl Delays {
    /// Takes the delay of one more message.
    pub fn add(&mut self, delay_s: f64) {
        self.count += 1;
        self.sum_s += delay_s;
        if delay_s > 0.001 {
            self.over_1ms += 1;
        }
        self.worst_s = self.worst_s.max(delay_s);
    }

    /// For example `mean 0.08 ms, 0.12 % above 1 ms, worst 4.31 ms`.
    pub fn describe(&self) -> String {
        let count = self.count.max(1) as f64;
        format!(
            "mean {:.2} ms, {:.2} % above 1 ms, worst {:.2} ms",
            1000.0 * self.sum_s / count,
            100.0 * self.over_1ms as f64 / count,
            1000.0 * self.worst_s
        )
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_what_it_was_given() {
        let mut delays = Delays::default();
        assert_eq!(delays.describe(), "mean 0.00 ms, 0.00 % above 1 ms, worst 0.00 ms");
        for delay_s in [0.0, 0.0005, 0.0005, 0.003] {
            delays.add(delay_s);
        }
        assert_eq!(delays.describe(), "mean 1.00 ms, 25.00 % above 1 ms, worst 3.00 ms");
    }
}