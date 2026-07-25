// The screenshot executable is deliberately thin: production and tests share
// the library-owned browser/WebGPU runtime instead of maintaining an example-
// local host implementation.
fn main() {
    afterglow_shell::testing::browser_runner::run_from_args();
}
