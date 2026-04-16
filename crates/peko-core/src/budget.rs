use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone)]
pub struct IterationBudget {
    remaining: Arc<AtomicUsize>,
    max: usize,
    interrupted: Arc<AtomicBool>,
}

impl IterationBudget {
    pub fn new(max_iterations: usize) -> Self {
        Self {
            remaining: Arc::new(AtomicUsize::new(max_iterations)),
            max: max_iterations,
            interrupted: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn decrement(&self) -> Result<usize, BudgetExhausted> {
        let prev = self.remaining.fetch_sub(1, Ordering::SeqCst);
        if prev == 0 {
            self.remaining.store(0, Ordering::SeqCst);
            Err(BudgetExhausted)
        } else {
            Ok(prev - 1)
        }
    }

    pub fn remaining(&self) -> usize {
        self.remaining.load(Ordering::SeqCst)
    }

    pub fn interrupt(&self) {
        self.interrupted.store(true, Ordering::SeqCst);
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupted.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.remaining.store(self.max, Ordering::SeqCst);
        self.interrupted.store(false, Ordering::SeqCst);
    }

    pub fn should_stop(&self) -> bool {
        self.is_interrupted() || self.remaining() == 0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("iteration budget exhausted")]
pub struct BudgetExhausted;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decrement() {
        let budget = IterationBudget::new(3);
        assert_eq!(budget.remaining(), 3);
        assert_eq!(budget.decrement().unwrap(), 2);
        assert_eq!(budget.decrement().unwrap(), 1);
        assert_eq!(budget.decrement().unwrap(), 0);
        assert!(budget.decrement().is_err());
    }

    #[test]
    fn test_interrupt() {
        let budget = IterationBudget::new(10);
        assert!(!budget.is_interrupted());
        budget.interrupt();
        assert!(budget.is_interrupted());
        assert!(budget.should_stop());
    }

    #[test]
    fn test_reset() {
        let budget = IterationBudget::new(5);
        budget.decrement().unwrap();
        budget.interrupt();
        budget.reset();
        assert_eq!(budget.remaining(), 5);
        assert!(!budget.is_interrupted());
    }

    #[test]
    fn test_cross_thread_interrupt() {
        let budget = IterationBudget::new(100);
        let budget2 = budget.clone();
        let handle = std::thread::spawn(move || {
            budget2.interrupt();
        });
        handle.join().unwrap();
        assert!(budget.is_interrupted());
    }
}
