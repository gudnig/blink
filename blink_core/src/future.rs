use std::{future::Future, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll, Waker}};

use crate::value::BlinkValue;

#[derive(Clone)]
pub struct BlinkFuture {
    inner: Arc<Mutex<FutureState>>,
}

enum FutureState {
    Pending { waker: Option<Waker> },
    RustFuture {
        future: Pin<Box<dyn Future<Output = Result<BlinkValue, String>> + Send>>,
    },
    Ready(Result<BlinkValue, String>),
}

impl Future for BlinkFuture {
    type Output = Result<BlinkValue, String>;
    
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            FutureState::Pending { waker} => {
                *waker = Some(cx.waker().clone());
                Poll::Pending
            },

            FutureState::RustFuture { future } => {
                future.as_mut().poll(cx)
            },
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
                *state = FutureState::Ready(Ok(value));
                
                // Wake up any waiting tasks
                if let Some(waker) = old_waker {
                    waker.wake();
                }
                Ok(())
            }
            _ => Err("Future already completed".to_string()),
        }
    }
    pub fn from_rust_future(future: Pin<Box<dyn Future<Output = Result<BlinkValue, String>> + Send>>) -> Self {
        let inner = Arc::new(Mutex::new(FutureState::RustFuture { future }));
        Self { inner }
    }
    
    pub fn fail(&self, error: String) -> Result<(), String> {
        let mut state = self.inner.lock().unwrap();
        match &mut *state {
            FutureState::Pending { waker } => {
                let old_waker = waker.take();
                *state = FutureState::Ready(Err(error));
                
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