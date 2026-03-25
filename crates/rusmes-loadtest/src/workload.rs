//! Workload patterns for load testing

use std::time::{Duration, Instant};

/// Workload pattern
#[derive(Debug, Clone)]
pub enum WorkloadPattern {
    /// Steady load - constant rate
    Steady { rate: u64 },

    /// Spike test - sudden increase
    Spike {
        baseline: u64,
        peak: u64,
        spike_duration: Duration,
        spike_start: Duration,
    },

    /// Ramp-up - gradual increase
    RampUp {
        start_rate: u64,
        end_rate: u64,
        duration: Duration,
    },

    /// Stress test - find breaking point
    Stress {
        start_rate: u64,
        increment: u64,
        interval: Duration,
    },

    /// Wave pattern - oscillating load
    Wave {
        min_rate: u64,
        max_rate: u64,
        period: Duration,
    },
}

impl WorkloadPattern {
    /// Get the target rate at a given time
    pub fn rate_at(&self, elapsed: Duration) -> u64 {
        match self {
            WorkloadPattern::Steady { rate } => *rate,

            WorkloadPattern::Spike {
                baseline,
                peak,
                spike_duration,
                spike_start,
            } => {
                if elapsed >= *spike_start && elapsed < *spike_start + *spike_duration {
                    *peak
                } else {
                    *baseline
                }
            }

            WorkloadPattern::RampUp {
                start_rate,
                end_rate,
                duration,
            } => {
                if elapsed >= *duration {
                    *end_rate
                } else {
                    let progress = elapsed.as_secs_f64() / duration.as_secs_f64();
                    let rate_diff = *end_rate as f64 - *start_rate as f64;
                    (*start_rate as f64 + rate_diff * progress) as u64
                }
            }

            WorkloadPattern::Stress {
                start_rate,
                increment,
                interval,
            } => {
                let intervals = elapsed.as_secs() / interval.as_secs();
                *start_rate + (*increment * intervals)
            }

            WorkloadPattern::Wave {
                min_rate,
                max_rate,
                period,
            } => {
                let progress =
                    (elapsed.as_secs_f64() % period.as_secs_f64()) / period.as_secs_f64();
                let amplitude = (*max_rate - *min_rate) as f64 / 2.0;
                let center = (*min_rate + *max_rate) as f64 / 2.0;
                let rate = center + amplitude * (progress * 2.0 * std::f64::consts::PI).sin();
                rate as u64
            }
        }
    }

    /// Calculate delay between requests for the given rate
    pub fn delay_for_rate(rate: u64) -> Duration {
        match 1_000_000u64.checked_div(rate) {
            Some(micros) => Duration::from_micros(micros),
            None => Duration::from_secs(1),
        }
    }
}

/// Workload controller
pub struct WorkloadController {
    pattern: WorkloadPattern,
    start_time: Instant,
}

impl WorkloadController {
    /// Create a new workload controller
    pub fn new(pattern: WorkloadPattern) -> Self {
        Self {
            pattern,
            start_time: Instant::now(),
        }
    }

    /// Get current target rate
    pub fn current_rate(&self) -> u64 {
        let elapsed = self.start_time.elapsed();
        self.pattern.rate_at(elapsed)
    }

    /// Get delay until next request
    pub fn next_delay(&self) -> Duration {
        let rate = self.current_rate();
        WorkloadPattern::delay_for_rate(rate)
    }

    /// Reset the start time
    pub fn reset(&mut self) {
        self.start_time = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_steady_workload() {
        let pattern = WorkloadPattern::Steady { rate: 100 };
        assert_eq!(pattern.rate_at(Duration::from_secs(0)), 100);
        assert_eq!(pattern.rate_at(Duration::from_secs(10)), 100);
        assert_eq!(pattern.rate_at(Duration::from_secs(100)), 100);
    }

    #[test]
    fn test_spike_workload() {
        let pattern = WorkloadPattern::Spike {
            baseline: 100,
            peak: 1000,
            spike_duration: Duration::from_secs(10),
            spike_start: Duration::from_secs(5),
        };

        assert_eq!(pattern.rate_at(Duration::from_secs(0)), 100);
        assert_eq!(pattern.rate_at(Duration::from_secs(7)), 1000);
        assert_eq!(pattern.rate_at(Duration::from_secs(20)), 100);
    }

    #[test]
    fn test_rampup_workload() {
        let pattern = WorkloadPattern::RampUp {
            start_rate: 100,
            end_rate: 1000,
            duration: Duration::from_secs(10),
        };

        assert_eq!(pattern.rate_at(Duration::from_secs(0)), 100);
        assert_eq!(pattern.rate_at(Duration::from_secs(5)), 550);
        assert_eq!(pattern.rate_at(Duration::from_secs(10)), 1000);
        assert_eq!(pattern.rate_at(Duration::from_secs(20)), 1000);
    }

    #[test]
    fn test_stress_workload() {
        let pattern = WorkloadPattern::Stress {
            start_rate: 100,
            increment: 50,
            interval: Duration::from_secs(10),
        };

        assert_eq!(pattern.rate_at(Duration::from_secs(0)), 100);
        assert_eq!(pattern.rate_at(Duration::from_secs(10)), 150);
        assert_eq!(pattern.rate_at(Duration::from_secs(20)), 200);
        assert_eq!(pattern.rate_at(Duration::from_secs(30)), 250);
    }

    #[test]
    fn test_wave_workload() {
        let pattern = WorkloadPattern::Wave {
            min_rate: 100,
            max_rate: 500,
            period: Duration::from_secs(60),
        };

        let rate_0 = pattern.rate_at(Duration::from_secs(0));
        let rate_15 = pattern.rate_at(Duration::from_secs(15));
        let rate_30 = pattern.rate_at(Duration::from_secs(30));

        // At 0 seconds, sine wave is at 0 (center)
        assert!((rate_0 as i64 - 300).abs() < 10);
        // At 15 seconds (quarter period), sine wave is at peak
        assert!(rate_15 > 400);
        // At 30 seconds (half period), sine wave is back at center
        assert!((rate_30 as i64 - 300).abs() < 10);
    }

    #[test]
    fn test_delay_calculation() {
        let delay_100 = WorkloadPattern::delay_for_rate(100);
        assert_eq!(delay_100, Duration::from_micros(10_000));

        let delay_1000 = WorkloadPattern::delay_for_rate(1000);
        assert_eq!(delay_1000, Duration::from_micros(1_000));
    }

    #[test]
    fn test_workload_controller() {
        let pattern = WorkloadPattern::Steady { rate: 100 };
        let controller = WorkloadController::new(pattern);

        assert_eq!(controller.current_rate(), 100);
        assert_eq!(controller.next_delay(), Duration::from_micros(10_000));
    }

    #[test]
    fn test_workload_controller_reset() {
        let pattern = WorkloadPattern::RampUp {
            start_rate: 100,
            end_rate: 1000,
            duration: Duration::from_secs(10),
        };
        let mut controller = WorkloadController::new(pattern);

        std::thread::sleep(Duration::from_millis(100));
        let rate_before = controller.current_rate();

        controller.reset();
        let rate_after = controller.current_rate();

        // After reset, rate should be back to start_rate
        assert!(rate_after < rate_before || rate_before == 100);
    }
}
