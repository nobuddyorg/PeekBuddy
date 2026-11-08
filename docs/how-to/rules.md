# Get told when something specific happens

A rule watches the stream of changes and tells you when it sees a pattern: a
config file changed, a folder saw a burst of deletes. Everything stays on this
PC: a toast, a tray balloon, or a program you chose. There is no email, no
webhook and no network action.

**Status.** Rules are evaluated while the engine runs. The notification is
a tray balloon for now; a native toast and the settings editor arrive with
the next parts of #81. A program you chose runs as described below.

Rules live in `settings.toml` as `[[rule]]` tables (all keys are in the
[settings reference](../reference/settings.md)).

```toml
[[rule]]
name = "Config changed"
watches = ["docs"]          # watch ids; leave out for every watch
glob = "*.toml"             # leave out for every path
kinds = ["modified"]        # leave out for every kind of file change
threshold = 1               # matches needed ...
window_s = 10               # ... within this many seconds
cooldown_s = 60             # quiet time after the rule fired
```

## When a rule fires

A rule fires when `threshold` matching changes fall within `window_s`
seconds. After it fires it stays quiet for `cooldown_s`; the matches that
come meanwhile are only counted, and the next notification says "N more
matches". A rule never changes or hides the events it looks at, and a slow
notifier never slows capture: when the queue to it is full the notification is
dropped and counted as suppressed.

## Run a program

```toml
[[rule]]
name = "Mass delete"
kinds = ["deleted"]
threshold = 50
window_s = 10
toast = false

[rule.command]
argv = ['C:\tools\alert.exe', "%path%", "%kind%"]
timeout_s = 15
confirmed = true
```

**Read this before you turn it on.** File names are chosen by whoever writes
into the watched folders, and a rule hands them to your program. PeekBuddy
protects the command line: the program is started directly (never through
`cmd.exe`), `%path%`, `%kind%`, `%root%` and `%ts%` are only replaced when they
are a whole argument, and each one stays exactly one argument whatever the
name contains. What your program does with that argument is up to it: do not
pass it on to a shell, do not delete or run what it names without checking it.
A program that writes into a watched folder can fire its own rule; the
cooldown and a one-instance-per-rule limit stop that from running away.

`confirmed = true` is required; without it the file is refused.

The program must be a full path. It runs with your rights, in a job object
that is killed after `timeout_s` and when PeekBuddy stops.
