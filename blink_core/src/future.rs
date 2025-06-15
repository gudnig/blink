use std::{future::Future, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll, Waker}};

use crate::{error::{BlinkError, LispError}, value::BlinkValue};

#[derive(Debug, Clone)]
pub struct BlinkFuture {
    inner: Arc<Mutex<FutureState>>,
}

enum FutureState {
    Pending { waker: Option<Waker> },
    RustFuture {
        future: Pin<Box<dyn Future<Output = BlinkValue> + Send>>,
    },
    Ready(BlinkValue),
}


// Make sure FutureState is Send + Sync
unsafe impl Send for FutureState {}
unsafe impl Sync for FutureState {}

impl Future for BlinkFuture {
    type Output = BlinkValue;
    
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
            FutureState::Ready(blink_value) => {
                Poll::Ready(blink_value.clone())
            },
        }
    }
}

impl BlinkFuture {
    pub fn new() -> Self { 
        let inner = Arc::new(Mutex::new(FutureState::Pending { waker: None }));
        Self { inner }
    }
    pub fn complete(&self, value: BlinkValue) -> Result<(), String> {
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
    pub fn from_rust_future(future: Pin<Box<dyn Future<Output = BlinkValue> + Send>>) -> Self {
        let inner = Arc::new(Mutex::new(FutureState::RustFuture { future }));
        Self { inner }
    }
    
    pub fn fail(&self, error: String) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            FutureState::Pending { waker } => {
                let old_waker = waker.take();
                *state = FutureState::Ready(BlinkError::eval(error).into_blink_value());
                
                if let Some(waker) = old_waker {
                    waker.wake();
                }
                Ok(())
            }
            _ => Err("Future already completed".to_string()),
        }
    }
    
    pub fn is_completed(&self) -> bool {
        matches!(*self.inner.lock().unwrap(), FutureState::Ready(_))
    }
}