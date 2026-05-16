use log::error;
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub(crate) fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    match lock.read() {
        Ok(guard) => guard,
        Err(poisoned) => {
            error!("[lock] RwLock poisoned on read — a thread panicked while holding this lock. Recovering with potentially inconsistent data.");
            poisoned.into_inner()
        }
    }
}

pub(crate) fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    match lock.write() {
        Ok(guard) => guard,
        Err(poisoned) => {
            error!("[lock] RwLock poisoned on write — a thread panicked while holding this lock. Recovering with potentially inconsistent data.");
            poisoned.into_inner()
        }
    }
}

pub(crate) fn mutex_lock<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    match lock.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            error!("[lock] Mutex poisoned — a thread panicked while holding this lock. Recovering with potentially inconsistent data.");
            poisoned.into_inner()
        }
    }
}
