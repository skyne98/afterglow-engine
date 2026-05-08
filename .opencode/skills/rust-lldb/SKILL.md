---
name: rust-lldb
description: Debug Rust programs using rust-lldb via a persistent tmux session. Set breakpoints, step, inspect variables, and compare against expected values.
---

## Setup

Requires `rust-lldb` and `tmux`. Build the target with debug symbols: `cargo build`

## Start a debug session

```
tmux new-session -d -s lldb 'rust-lldb /path/to/binary'
sleep 5   # wait for lldb + Rust imports to initialize
```

## Set breakpoints

By function name:
```
tmux send-keys -t lldb 'b handle' Enter
sleep 1
```

By file and line:
```
tmux send-keys -t lldb 'breakpoint set --file main.rs --line 8' Enter
sleep 1
```

## Run the program

```
tmux send-keys -t lldb 'run' Enter
sleep 2
```

If the program waits for input (e.g., a server accepting connections), send it from another command:
```
echo "message" | nc --send-only 127.0.0.1 <port>
sleep 2
```

## Step through code

```
tmux send-keys -t lldb 'next' Enter
sleep 1
```

Step three times to reach `println!` where all variables are in scope:
```
tmux send-keys -t lldb 'next' Enter
sleep 1
tmux send-keys -t lldb 'next' Enter
sleep 1
tmux send-keys -t lldb 'next' Enter
sleep 1
```

## Inspect variables

```
tmux send-keys -t lldb 'expr n' Enter
sleep 0.5
tmux send-keys -t lldb 'expr msg' Enter
sleep 0.5
tmux capture-pane -t lldb -p -S -10
```

## Continue execution

```
tmux send-keys -t lldb 'continue' Enter
```

## Read output

```
tmux capture-pane -t lldb -p -S -<N>
```

## Kill the session

```
tmux kill-session -t lldb
```

## Full server debug example

```
# Build
cargo build

# Start lldb in tmux
tmux new-session -d -s lldb 'rust-lldb target/debug/my-server'
sleep 5

# Set breakpoint and run
tmux send-keys -t lldb 'b handle' Enter
sleep 1
tmux send-keys -t lldb 'run' Enter
sleep 2

# Send test message
echo "test payload" | nc --send-only 127.0.0.1 9877
sleep 2

# Step and inspect
tmux send-keys -t lldb 'next' Enter
sleep 1
tmux send-keys -t lldb 'next' Enter
sleep 1
tmux send-keys -t lldb 'next' Enter
sleep 1
tmux send-keys -t lldb 'expr n' Enter
sleep 0.5
tmux send-keys -t lldb 'expr msg' Enter
sleep 0.5

# Read results
tmux capture-pane -t lldb -p -S -15

# Continue or quit
tmux send-keys -t lldb 'quit' Enter
sleep 0.5
tmux kill-session -t lldb
```

## Best practices

- Always wait 5s after starting lldb for Rust type imports to complete.
- Use `-S -N` to read only the last N lines (lldb output is verbose).
- Step to `println!` or equivalent before inspecting — local variables must be in scope.
- Kill the tmux session when done.
- For interactive exploration, leave the session alive and send commands as needed.
