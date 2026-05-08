---
name: tmux-session
description: Spawn, interact with, and manage persistent terminal sessions using tmux. Send commands and read output on demand.
---

## Setup

Requires `tmux`. Check: `tmux -V`

## Create a session

```
tmux new-session -d -s <name> '<command>'
```

The session runs in background. Commands and output are exchanged on demand.

## Send commands

```
tmux send-keys -t <name> '<command>' Enter
```

Wait briefly after for the command to execute before reading output.

## Read output

```
tmux capture-pane -t <name> -p
```

Last N lines:
```
tmux capture-pane -t <name> -p -S -<N>
```

## Full lifecycle example

```
# Start
tmux new-session -d -s mysess 'python3 -i'
sleep 1

# Send command, read result
tmux send-keys -t mysess '2 + 2' Enter
sleep 0.5
tmux capture-pane -t mysess -p -S -5

# Send another, read again
tmux send-keys -t mysess 'import os; os.listdir(".")' Enter
sleep 0.5
tmux capture-pane -t mysess -p -S -10

# Clean up
tmux kill-session -t mysess
```

## List sessions

```
tmux ls
```

## Kill a session

```
tmux kill-session -t <name>
```

## Best practices

- Always `sleep` after `send-keys` to let the command execute.
- Use `-S -N` to read only the last N lines of output.
- Kill sessions when done to avoid accumulation.
- Use distinct session names to avoid conflicts.
- If a session name already exists, kill it first or use a different name.
