use super::*;
use std::time::Instant;

pub(super) fn retry<T>(
    mut operation: impl FnMut() -> Result<T, StoreError>,
) -> Result<T, StoreError> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        match operation() {
            Err(StoreError::Held) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            result => return result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn retries_only_transient_transaction_ownership() {
        let held_attempts = Cell::new(0_u8);
        let adopted = retry(|| {
            let attempt = held_attempts.get().saturating_add(1);
            held_attempts.set(attempt);
            if attempt < 3 {
                Err(StoreError::Held)
            } else {
                Ok("adopted")
            }
        });
        assert!(matches!(adopted, Ok("adopted")));
        assert_eq!(held_attempts.get(), 3);

        let mismatch_attempts = Cell::new(0_u8);
        let mismatch = retry::<()>(|| {
            mismatch_attempts.set(mismatch_attempts.get().saturating_add(1));
            Err(StoreError::TokenMismatch)
        });
        assert!(matches!(mismatch, Err(StoreError::TokenMismatch)));
        assert_eq!(mismatch_attempts.get(), 1);
    }
}
