use std::{future::Future, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll, Waker}};

use crate::value::ValueRef;

#[derive(Debug, Clone)]
pub struct BlinkFuture {
    inner: Arc<Mutex<FutureState>>,
}


enum FutureState {
    Pending { waker: Option<Waker> },
    RustFuture {
        future: Pin<Box<dyn Future<Output = ValueRef> + Send>>,
    },
    Ready(ValueRef),
}

impl std::fmt::Debug for FutureState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FutureState::Pending { waker } => {
                f.debug_struct("Pending")
                    .field("waker", &waker.is_some())
                    .finish()
            }
            FutureState::RustFuture { .. } => {
                f.debug_struct("RustFuture")
                    .field("future", &"<dyn Future>")
                    .finish()
            }
            FutureState::Ready(value) => {
                f.debug_struct("Ready")
                    .field("value", value)
                    .finish()
            }
        }
    }
}

// Make sure FutureState is Send + Sync
unsafe impl Send for FutureState {}
unsafe impl Sync for FutureState {}

impl Future for BlinkFuture {
    type Output = ValueRef;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            FutureState::Pending { waker} => {
                *waker = Some(cx.waker().clone());
                Poll::Pending
            },

            FutureState::RustFuture { future } => {
                match future.as_mut().poll(cx) {
                    Poll::Ready(res) => {
                        *state = FutureState::Ready(res.clone());
                        Poll::Ready(res)
                    }
                    Poll::Pending => Poll::Pending,
                }
            }
            FutureState::Ready(value) => {
                Poll::Ready(*value)
            },
        }
    }
}

impl BlinkFuture {
    pub fn new() -> Self { 
        let inner = Arc::new(Mutex::new(FutureState::Pending { waker: None }));
        Self { inner }
    }
    pub fn complete(&self, value: ValueRef) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            FutureState::Pending { waker } => {
                let old_waker = waker.take();
                *state = FutureState::Ready(value);
                
                // Wake up any waiting tasks
                if let Some(waker) = old_waker {
                    waker.wake();
                }
                Ok(())
            }
            _ => Err("Future already completed".to_string()),
        }
    }
    pub fn from_rust_future(future: Pin<Box<dyn Future<Output = ValueRef> + Send>>) -> Self {
        let inner = Arc::new(Mutex::new(FutureState::RustFuture { future }));
        Self { inner }
    }

    
    pub fn is_completed(&self) -> bool {
        matches!(*self.inner.lock().unwrap(), FutureState::Ready(_))
    }
}