use std::sync::{Arc, Mutex};
use std::collections::{VecDeque};
use std::time::Duration;
use errors::*;
use log::*;
use utils::*;
use api::Api;
use api_objects::*;
use worker_utils::*;

use futures::Future;
use futures::future::BoxFuture;
use tokio_timer::Timer;
use robots::actors::*;

use workers::GetChunkAnswer::*;

pub type Pos = i32;

#[derive(Clone)]
pub enum M<C> {
  GetChunkAtPos(Pos),
  PChunkReceived(Pos, Vec<C>),
  PWorkTimedOut(Pos)
}

#[derive(Clone)]
pub enum GetChunkAnswer<T> {
  HasChunk(T),
  NoChunk,
  CacheFull
}

enum Work {
  DownloadingAtPos(Pos),
  Idle
}

struct Linear<T> {
  work: Work,
  cache: Vec<T>,
  cache_full: bool
}

struct FriendsWorker {
  chunk_size: i32,
  api: Arc<Api>,
  main: Mutex<Linear<User>>,
}

impl FriendsWorker {
  fn new(api: &Arc<Api>) -> Self {
    FriendsWorker {
      chunk_size: 100,
      api: api.clone(),
      main: Mutex::new(
        Linear {
          work: Work::Idle,
          cache: Vec::new(),
          cache_full: false
        }
      )
    }
  }
}

impl Actor for FriendsWorker {
  fn receive(&self, msg: Box<Any>, context: ActorCell) {
    const PREF: Rstr = "friends:";
    type T = User;
    if let (Ok(msg), Ok(mut c))
    = (Box::<Any>::downcast::<M<T>>(msg), self.main.lock()) {
      match msg {
        box M::GetChunkAtPos(pos) => {
          let reqrange = (pos + self.chunk_size) as usize;
          let upos = pos as usize;

          let answ = if c.cache_full {
            CacheFull
          } else if reqrange < c.cache.len() {
            HasChunk(c.cache[upos..reqrange].to_vec())
          } else {
            match c.work {
              Work::Idle if upos <= c.cache.len() => {
                let apiwork = self.api
                  .as_ref()
                  .friends_get(self.chunk_size, pos)
                  .boxed();

                fork(apiwork, Duration::from_secs(4000), &context, pos, &PREF);
                c.work = Work::DownloadingAtPos(pos);
                debug!("{} task started at pos {}", PREF, pos);
                NoChunk
              },
              _ => NoChunk
            }
          };
          context.complete(context.sender(), answ)
        },

        box M::PWorkTimedOut(wp) => {
          match c.work {
            Work::DownloadingAtPos(p) if wp == p => {
              warn!("{} task timed out at pos {}", PREF, p);
              c.work = Work::Idle
            },
            _ =>
              warn!("{} late timeout \
              with pos: {}", PREF, wp)
          }
        },

        box M::PChunkReceived(pos, chunk) => {
          let upos = pos as usize;
          match c.work {
            Work::DownloadingAtPos(p)
            if pos == p && upos <= c.cache.len() => {
              if chunk.len() == 0 {
                info!("{} cache full (received empty chunk)", PREF);
                c.cache_full = true;
              } else {
                let mut ln = c.cache.len();
                while upos < ln {
                  c.cache.remove(ln - 1);
                  ln = c.cache.len();
                }
                c.cache.extend_from_slice(&chunk[..]);
              }
              debug!("{} chunk received to pos {}({})", PREF, pos, chunk.len());
              c.work = Work::Idle;
            },
            _ =>
              warn!("{} incorrect chunk \
              \npos: {}, lcache: {}", PREF, pos, c.cache.len())
          }
        },

        _ => ()
      }
    } else {
      error!("{} can't acquire lck / downcast message", PREF);
    }
  }
}