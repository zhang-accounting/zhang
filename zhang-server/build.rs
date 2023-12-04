use chrono::Local;

fn main() {
    let now = Local::now();
    println!("cargo:rustc-env=ZHANG_BUILD_DATE={}", now.format("%Y-%m-%d"));
}
