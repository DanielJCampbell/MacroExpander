macro_rules! bar {
  () => { let x = 1; }
}

macro_rules! foo {
  () => { bar!(); }
}

fn main() {
  foo!();
}