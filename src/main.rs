use state_store::StateStore;
use termination::create_termination;
use ui_management::UiManager;

mod state_store;
mod termination;
mod ui_management;

use termination::{Interrupted, Terminator};

use log::{info, trace};

use std::collections::VecDeque;
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use std::io;
use std::{thread, time};

use log::LevelFilter;
use log::*;

#[cfg(feature = "crossterm")]
use crossterm::event::KeyCode as Key;
#[cfg(feature = "crossterm")]
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

#[cfg(feature = "termion")]
use termion::{
    event::{Event, Key},
    input::{MouseTerminal, TermRead},
    raw::IntoRawMode,
    screen::AlternateScreen,
};

#[cfg(feature = "examples-ratatui-crossterm")]
use ratatui::backend::CrosstermBackend as SelectedBackend;
#[cfg(feature = "examples-ratatui-termion")]
use ratatui::backend::TermionBackend as SelectedBackend;
use ratatui::prelude::*;
use ratatui::widgets::*;

use tui_logger::*;

struct App {
    states: Vec<TuiWidgetState>,
    tabs: Vec<String>,
    selected_tab: usize,
    opt_info_cnt: Option<u16>,
}

#[derive(Debug)]
enum AppEvent {
    UiEvent(Event),
    LoopCnt(Option<u16>),
}

fn demo_application(tx: mpsc::Sender<AppEvent>) {
    let one_second = time::Duration::from_millis(1_000);
    let mut lp_cnt = (1..=100).into_iter();
    loop {
        trace!(target:"DEMO", "Sleep one second");
        thread::sleep(one_second);
        trace!(target:"DEMO", "Issue log entry for each level");
        error!(target:"error", "an error");
        warn!(target:"warn", "a warning");
        trace!(target:"trace", "a trace");
        debug!(target:"debug", "a debug");
        info!(target:"info", "an info");
        tx.send(AppEvent::LoopCnt(lp_cnt.next())).unwrap();
    }
}


/// Create a WorkQueue of any type that holds all the work to be done
#[derive(Clone)]
struct WorkQueue<T> {
    queue: Arc<Mutex<VecDeque<T>>>,
}

impl<T> WorkQueue<T> {
    /// Create a new empty queue
    fn new() -> Self {
        Self {
            queue: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Add work to the queue
    fn add_work(&self, work: T) -> Result<(), ()> {
        let queue = self.queue.lock();

        if let Ok(mut q) = queue {
            q.push_back(work);
            Ok(())
        } else {
            Err(())
        }
    }

    /// Get the first available work
    fn get_work(&self) -> Option<T> {
        // Lock the queue to fetch a work to do and prevent other threads from
        // fetching the same work.
        let queue = self.queue.lock();

        if let Ok(mut q) = queue {
            // Remove the first work available
            // Follows the the FIFO layout
            q.pop_front()
        } else {
            None
        }
    }

    /// Count the work left
    fn length(&self) -> Option<usize> {
        let queue = self.queue.lock();

        if let Ok(q) = queue {
            Some(q.len())
        } else {
            None
        }
    }
}

/// A very complex calculation that takes too much time to execute
async fn calculate_y(y: i32, duration: u64) -> i32 {
    trace!("worker_y:calculate");

    // Use tokio::time::sleep instead of thread::sleep to avoid blocking the
    // entire thread.
    tokio::time::sleep(Duration::from_millis(duration)).await;
    y * 2
}

/// A very complex calculation that takes too much time to execute
async fn calculate_x(x: i32, duration: u64) -> i32 {
    trace!("worker_x:calculate");

    // Use tokio::time::sleep instead of thread::sleep to avoid blocking the
    // entire thread.
    tokio::time::sleep(Duration::from_millis(duration)).await;
    x * 2
}

async fn create_worker_y(
    i: u32,
    queue_clone: WorkQueue<i32>,
    max_work_async: i32,
    tx_clone: mpsc::Sender<i32>,
) {
    // How much work has this thread done
    let mut work_done: i32 = 0;
    let mut current_work: i32 = 0;

    // Check if there is more work to be done
    while queue_clone.length().unwrap() > 0 {
        trace!("worker_y:check_work_avail");
        let mut tasks = Vec::new();

        while current_work < max_work_async {
            if let Some(work) = queue_clone.get_work() {
                trace!("worker_y:get_work");
                let task = tokio::task::spawn(calculate_x(work, 1000));
                tasks.push(task);
                work_done += 1;
                current_work += 1;
            } else {
                break;
            }
        }

        trace!("worker_y:wait_for_task_completion");
        for task in tasks {
            let result = task.await.unwrap();
            tx_clone.send(result).unwrap();
        }

        current_work = 0;
    }

    trace!("worker_y:thread:{:?}:work_done:{:?}", i, work_done);
}
async fn create_worker_x(
    i: u32,
    queue_clone: WorkQueue<i32>,
    max_work_async: i32,
    tx_clone: mpsc::Sender<i32>,
) {
    // How much work has this thread done
    let mut work_done: i32 = 0;
    let mut current_work: i32 = 0;

    // Check if there is more work to be done
    while queue_clone.length().unwrap() > 0 {
        trace!("worker_x:check_work");
        let mut tasks = Vec::new();

        while current_work < max_work_async {
            if let Some(work) = queue_clone.get_work() {
                trace!("worker_x:get_work");
                let task = tokio::task::spawn(calculate_y(work, 1000));
                tasks.push(task);
                work_done += 1;
                current_work += 1;
            } else {
                break;
            }
        }

        trace!("worker_x:wait_for_task_completion");
        for task in tasks {
            let result = task.await.unwrap();
            tx_clone.send(result).unwrap();
        }

        current_work = 0;
    }

    trace!("worker_x:thread:{:?}:work_done:{:?}", i, work_done);
}

#[tokio::main]
async fn chat() -> anyhow::Result<()> {
    let (terminator, mut interrupt_rx) = create_termination();
    let (state_store, state_rx) = StateStore::new();
    let (ui_manager, action_rx) = UiManager::new();

    tokio::try_join!(
        state_store.main_loop(terminator, action_rx, interrupt_rx.resubscribe()),
        ui_manager.main_loop(state_rx, interrupt_rx.resubscribe()),
    )?;

    if let Ok(reason) = interrupt_rx.recv().await {
        match reason {
            Interrupted::UserInt => println!("exited per user request"),
            Interrupted::OsSigInt => println!("exited because of an os sig int"),
        }
    } else {
        println!("exited because of an unexpected error");
    }

    Ok(())
}
#[macro_use]
extern crate log;
//use tui_logger;
fn main() -> anyhow::Result<()> {

    // Set max_log_level to Trace
    tui_logger::init_logger(log::LevelFilter::Trace).unwrap();

    // Set default level for unknown targets to Trace
    tui_logger::set_default_level(log::LevelFilter::Trace);

    chat()
}
