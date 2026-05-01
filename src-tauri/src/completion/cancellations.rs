//! Per-request cancellation flags for the completion pipeline. Sources
//! whose `fetch` work isn't sub-ms (today: ripgrep) check the flag
//! between matches and halt the walk early when a newer query
//! supersedes the in-flight one.
//!
//! Lives in Tauri managed state alongside the `CompletionRegistry`;
//! the Tauri command surface (`completion::commands::*`) writes to
//! it. No socket-RPC surface — completion is webview-only and never
//! needed a JSON-RPC mirror per the K-XXX MR review.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct CompletionCancellations {
    inner: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl CompletionCancellations {
    pub fn new_token(&self, request_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(request_id.to_string(), Arc::clone(&flag));
        }
        flag
    }

    pub fn cancel(&self, request_id: &str) -> bool {
        let mut found = false;
        if let Ok(mut guard) = self.inner.lock() {
            if let Some(flag) = guard.remove(request_id) {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
                found = true;
            }
        }
        found
    }

    pub fn forget(&self, request_id: &str) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.remove(request_id);
        }
    }
}
