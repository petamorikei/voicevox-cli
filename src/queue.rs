//! Playback queue shared between vvsay processes.
//!
//! Processes started with `--queue` take a ticket at startup and wait for their turn before
//! playing, so they play in the order they were invoked. Processes without `--queue` never wait,
//! but they register while playing so that queued processes also wait for them.
//!
//! The state lives in a JSON file guarded by an exclusive file lock. Entries are tied to both the
//! vvsay process and its mpv player, and are discarded only when both are gone: mpv keeps playing
//! buffered audio even if vvsay is killed.

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub pid: u32,
    pub player_pid: Option<u32>,
    /// `None` for processes started without `--queue`.
    pub ticket: Option<u64>,
    pub playing: bool,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub next_ticket: u64,
    pub entries: Vec<Entry>,
}

impl State {
    /// Removes entries whose vvsay process and player have both exited.
    fn prune(&mut self, is_alive: impl Fn(u32) -> bool) {
        self.entries
            .retain(|e| is_alive(e.pid) || e.player_pid.is_some_and(&is_alive));
    }

    /// Whether the queued process `pid` may start playing now: nothing else is playing and no
    /// process with an earlier ticket is still waiting.
    pub fn is_turn(&self, pid: u32) -> bool {
        let Some(ticket) = self
            .entries
            .iter()
            .find(|e| e.pid == pid)
            .and_then(|e| e.ticket)
        else {
            return true;
        };
        !self
            .entries
            .iter()
            .any(|e| e.pid != pid && (e.playing || e.ticket.is_some_and(|other| other < ticket)))
    }
}

pub struct Queue {
    dir: PathBuf,
}

impl Queue {
    pub fn new() -> Result<Self> {
        let home = dirs::home_dir().context("Could not find home directory")?;
        Ok(Self::with_dir(
            home.join(".local").join("state").join("vvsay"),
        ))
    }

    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Runs `f` on the shared state while holding the lock, then saves the state.
    fn update<R>(&self, f: impl FnOnce(&mut State) -> R) -> Result<R> {
        fs::create_dir_all(&self.dir)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.dir.join("queue.lock"))?;
        lock.lock()?;

        let path = self.dir.join("queue.json");
        let mut state: State = match fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => return Err(e.into()),
        };
        state.prune(is_process_alive);
        let result = f(&mut state);

        let tmp = self.dir.join("queue.json.tmp");
        fs::write(&tmp, serde_json::to_vec(&state)?)?;
        fs::rename(&tmp, &path)?;
        drop(lock);
        Ok(result)
    }

    /// Takes a ticket for `pid`. Called at startup so that the order of invocation is kept even if
    /// later invocations finish synthesizing first.
    pub fn enqueue(&self, pid: u32) -> Result<()> {
        self.update(|state| {
            let ticket = state.next_ticket;
            state.next_ticket += 1;
            state.entries.retain(|e| e.pid != pid);
            state.entries.push(Entry {
                pid,
                player_pid: None,
                ticket: Some(ticket),
                playing: false,
            });
        })
    }

    pub fn is_enqueued(&self, pid: u32) -> Result<bool> {
        self.update(|state| state.entries.iter().any(|e| e.pid == pid))
    }

    /// Blocks until it is `pid`'s turn, then marks it as playing.
    pub fn wait_turn(&self, pid: u32) -> Result<Lease<'_>> {
        loop {
            let ready = self.update(|state| {
                if !state.is_turn(pid) {
                    return false;
                }
                match state.entries.iter_mut().find(|e| e.pid == pid) {
                    Some(entry) => entry.playing = true,
                    None => state.entries.push(Entry {
                        pid,
                        player_pid: None,
                        ticket: None,
                        playing: true,
                    }),
                }
                true
            })?;
            if ready {
                return Ok(Lease { queue: self, pid });
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Registers `pid` as playing without waiting (for processes started without `--queue`).
    pub fn start_playing(&self, pid: u32) -> Result<Lease<'_>> {
        self.update(|state| {
            state.entries.retain(|e| e.pid != pid);
            state.entries.push(Entry {
                pid,
                player_pid: None,
                ticket: None,
                playing: true,
            });
        })?;
        Ok(Lease { queue: self, pid })
    }
}

/// Registration of a playing process. The entry is removed when the lease is dropped.
pub struct Lease<'a> {
    queue: &'a Queue,
    pid: u32,
}

impl Lease<'_> {
    /// Records the mpv process so the entry outlives vvsay if mpv is still playing.
    pub fn set_player(&self, player_pid: u32) -> Result<()> {
        let pid = self.pid;
        self.queue.update(|state| {
            if let Some(entry) = state.entries.iter_mut().find(|e| e.pid == pid) {
                entry.player_pid = Some(player_pid);
            }
        })
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        let pid = self.pid;
        let _ = self
            .queue
            .update(|state| state.entries.retain(|e| e.pid != pid));
    }
}

pub fn is_process_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // Signal 0 only checks for existence. EPERM means the process exists but belongs to another user.
    let exists = unsafe { libc::kill(pid, 0) } == 0;
    exists || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, ticket: Option<u64>, playing: bool) -> Entry {
        Entry {
            pid,
            player_pid: None,
            ticket,
            playing,
        }
    }

    fn state(entries: Vec<Entry>) -> State {
        State {
            next_ticket: 100,
            entries,
        }
    }

    #[test]
    fn earliest_ticket_goes_first() {
        let s = state(vec![entry(1, Some(5), false), entry(2, Some(6), false)]);
        assert!(s.is_turn(1));
        assert!(!s.is_turn(2));
    }

    #[test]
    fn queued_waits_for_unqueued_playback() {
        let s = state(vec![entry(1, None, true), entry(2, Some(6), false)]);
        assert!(!s.is_turn(2));
    }

    #[test]
    fn queued_waits_for_queued_playback() {
        let s = state(vec![entry(1, Some(5), true), entry(2, Some(6), false)]);
        assert!(!s.is_turn(2));
    }

    #[test]
    fn unqueued_waiting_entries_do_not_block() {
        // An unqueued process that is not playing yet should not appear, but must not block anyway.
        let s = state(vec![entry(1, None, false), entry(2, Some(6), false)]);
        assert!(s.is_turn(2));
    }

    #[test]
    fn prune_keeps_entries_with_live_player() {
        let mut s = state(vec![
            Entry {
                pid: 1,
                player_pid: Some(10),
                ticket: Some(5),
                playing: true,
            },
            entry(2, Some(6), false),
            entry(3, Some(7), false),
        ]);
        // vvsay 1 was killed but its mpv (10) is still playing; process 2 is dead.
        s.prune(|pid| pid == 10 || pid == 3);
        assert_eq!(s.entries.iter().map(|e| e.pid).collect::<Vec<_>>(), [1, 3]);
        assert!(!s.is_turn(3));
    }

    #[test]
    fn queue_roundtrip_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let queue = Queue::with_dir(dir.path().to_path_buf());
        let me = std::process::id();
        queue.enqueue(me).unwrap();
        assert!(queue.is_enqueued(me).unwrap());
        let lease = queue.wait_turn(me).unwrap();
        lease.set_player(me).unwrap();
        drop(lease);
        assert!(!queue.is_enqueued(me).unwrap());
    }
}
