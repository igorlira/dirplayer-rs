use std::{collections::HashMap, sync::{Arc, Mutex}, time::Duration};
use itertools::Itertools;
use lazy_static::lazy_static;

pub struct ProfilingToken {
  name: String,
  start_time: u64,
  end_time: Option<u64>,
}

impl ProfilingToken {
  pub fn elapsed(&self) -> Option<std::time::Duration> {
    match self.end_time {
      Some(end_time) => Some(Duration::from_millis(end_time - self.start_time)),
      None => None,
    }
  }
}

pub struct PlayerProfiler {
  tokens: HashMap<u32, ProfilingToken>,
  token_id_counter: u32,
}

impl PlayerProfiler {
  pub fn new() -> PlayerProfiler {
    PlayerProfiler { tokens: HashMap::new(), token_id_counter: 0 }
  }

  pub fn start(&mut self, name: String) -> u32 {
    let id = self.token_id_counter;
    self.tokens.insert(id, ProfilingToken {
      name,
      start_time: chrono::Local::now().timestamp_millis() as u64,
      end_time: None,
    });
    self.token_id_counter += 1;
    id
  }

  pub fn end(&mut self, id: u32) {
    let token = self.tokens.get_mut(&id).unwrap();
    token.end_time = Some(chrono::Local::now().timestamp_millis() as u64);
    let elapsed = token.elapsed().unwrap();

    println!("{} took {:?}", token.name, elapsed);
  }

  pub fn report(&self) -> String {
    let mut result = String::new();
    let mut total_elapsed_by_name: HashMap<String, Duration> = HashMap::new();

    for token in self.tokens.values() {
      let token_elapsed = token.elapsed();
      if token_elapsed.is_none() {
        continue;
      }
      let token_elapsed = token_elapsed.unwrap();
      let elapsed_by_this_name = total_elapsed_by_name.get(
        &token.name
      ).map(|x| x.to_owned()).unwrap_or(Duration::from_millis(0));
      total_elapsed_by_name.insert(token.name.to_owned(), elapsed_by_this_name + token_elapsed);
    }

    let total_elapsed: Duration = total_elapsed_by_name.values().map(|x| x.to_owned()).sum();
    let total_ms = total_elapsed.as_millis();

    let names_sorted_by_elapsed = total_elapsed_by_name.iter().sorted_by(|a, b| b.1.cmp(a.1)).rev();
    for (name, elapsed) in names_sorted_by_elapsed {
      let elapsed_percent = (elapsed.as_millis() as f64 / total_ms as f64) * 100.0;

      result.push_str(&format!("{} took {:?} ({:.2}%)\n", name, elapsed, elapsed_percent));
    }
    result.push_str(&format!("Total: {:?}\n", total_elapsed));

    return result;
  }
}

lazy_static! {
  pub static ref PROFILER: Arc<Mutex<PlayerProfiler>> = Arc::new(Mutex::new(PlayerProfiler::new()));
}

#[allow(dead_code)]
pub fn start_profiling(name: String) -> u32 {
  let mut profiler = PROFILER.lock().unwrap();
  profiler.start(name)
}

#[allow(dead_code)]
pub fn end_profiling(id: u32) {
  let mut profiler = PROFILER.lock().unwrap();
  profiler.end(id)
}

pub fn get_profiler_report() -> String {
  let profiler = PROFILER.lock().unwrap();
  profiler.report()
}
