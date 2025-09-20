use std::sync::Arc;
use tokio::sync::mpsc;
use parking_lot::Mutex;

#[derive(Debug, Clone)]
pub enum OutputMessage {
    /// Output from a goroutine (just the message, REPL will add formatting)
    GoroutineOutput {
        goroutine_id: u32,
        message: String
    },
    /// Regular print output (not from goroutine)
    StandardOutput(String),
}

pub struct OutputManager {
    sender: mpsc::UnboundedSender<OutputMessage>,
    receiver: Arc<Mutex<mpsc::UnboundedReceiver<OutputMessage>>>,
}

impl OutputManager {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();

        Self {
            sender,
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }

    /// Get a handle that can be used to send output messages
    pub fn get_sender(&self) -> OutputSender {
        OutputSender {
            sender: self.sender.clone(),
        }
    }

    /// Flush all pending output messages to stdout
    /// Returns the number of messages processed
    pub fn flush_pending_output(&self) -> usize {
        let mut receiver = self.receiver.lock();
        let mut count = 0;

        // Process all available messages without blocking
        while let Ok(message) = receiver.try_recv() {
            self.print_message(message);
            count += 1;
        }

        count
    }

    /// Wait for and process a single output message
    /// This is useful for blocking until goroutines produce output
    pub async fn process_one_message(&self) -> Option<()> {
        let mut receiver = self.receiver.lock();

        if let Some(message) = receiver.recv().await {
            self.print_message(message);
            Some(())
        } else {
            None
        }
    }

    /// Process all pending messages and optionally wait for more
    pub async fn process_messages_until_quiet(&self, max_wait_ms: u64) -> usize {
        let mut count = 0;

        // First, flush all immediately available messages
        count += self.flush_pending_output();

        // Then wait a bit for any delayed messages (like from goroutines)
        let timeout = tokio::time::Duration::from_millis(max_wait_ms);
        let start = tokio::time::Instant::now();

        while start.elapsed() < timeout {
            let mut receiver = self.receiver.lock();

            // Wait up to 10ms for a message
            let short_timeout = tokio::time::Duration::from_millis(10);

            match tokio::time::timeout(short_timeout, receiver.recv()).await {
                Ok(Some(message)) => {
                    drop(receiver); // Release lock before printing
                    self.print_message(message);
                    count += 1;
                }
                Ok(None) => break, // Channel closed
                Err(_) => {
                    // Timeout - check if any more messages are immediately available
                    drop(receiver);
                    let additional = self.flush_pending_output();
                    count += additional;
                    if additional == 0 {
                        break; // No more messages, we're done
                    }
                }
            }
        }

        count
    }

    fn print_message(&self, message: OutputMessage) {
        match message {
            OutputMessage::GoroutineOutput { goroutine_id, message } => {
                println!("[go-{}] {}", goroutine_id, message);
            }
            OutputMessage::StandardOutput(output) => {
                println!("{}", output);
            }
        }
    }
}

/// A handle for sending output messages
/// This can be cloned and passed to goroutines
#[derive(Clone)]
pub struct OutputSender {
    sender: mpsc::UnboundedSender<OutputMessage>,
}

impl OutputSender {
    /// Print from a goroutine context
    pub fn print_from_goroutine(&self, goroutine_id: u32, message: &str) {
        let _ = self.sender.send(OutputMessage::GoroutineOutput {
            goroutine_id,
            message: message.to_string(),
        });
    }

    /// Send standard output (not from goroutine)
    pub fn print_standard(&self, message: &str) {
        let _ = self.sender.send(OutputMessage::StandardOutput(message.to_string()));
    }

    /// Check if the sender is still connected
    pub fn is_closed(&self) -> bool {
        self.sender.is_closed()
    }
}