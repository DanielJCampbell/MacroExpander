macro_rules! bar {
  () => { 1 }
}

macro_rules! foo {
  () => { bar!(); }
}

fn main() {
  let x = foo!();
}