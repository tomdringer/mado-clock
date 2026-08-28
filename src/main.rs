use std::io::Write;
  use chrono::Local;

  fn main() {
      loop {
          let now = Local::now();

          print!("\x1b[H\x1b[2J");  // move to top-left, clear screen
          println!();
          println!("  \x1b[1;38;2;148;163;184m{}\x1b[0m", now.format("%H:%M:%S"));
          println!("  \x1b[38;2;71;85;105m{}\x1b[0m",     now.format("%A"));
          println!("  \x1b[38;2;71;85;105m{}\x1b[0m",     now.format("%B %-d, %Y"));

          std::io::stdout().flush().ok();
          std::thread::sleep(std::time::Duration::from_secs(1));
      }
  }
