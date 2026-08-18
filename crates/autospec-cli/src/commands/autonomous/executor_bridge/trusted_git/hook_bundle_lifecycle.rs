use super::*;

pub(in crate::commands::autonomous::executor_bridge::trusted_git) static HOOK_BUNDLE_NONCE:
    AtomicU64 = AtomicU64::new(0);

impl Drop for TrustedHookBundle {
    fn drop(&mut self) {
        if self.temporary {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
