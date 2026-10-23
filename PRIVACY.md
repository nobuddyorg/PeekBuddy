# Privacy

No login, no server, no tracking; the log never leaves your disk.

- PeekBuddy makes no network connections: no telemetry, no update checks, no
  accounts.
- **What is stored**: paths, names, sizes, attributes and timestamps of changes
  in the folders you chose to watch. Never file contents.
- **Where**: `%LOCALAPPDATA%\nobuddy\PeekBuddy\`, or a `data` folder next to
  the exe in portable mode. The log is stored per Windows user and is not
  shared with other accounts. Folder names can themselves be sensitive; they
  stay in that per-user folder.
- **How to delete**: set a shorter retention in the settings, or quit
  PeekBuddy and delete the folder.
- **Exports** happen only when you trigger them, to the file you choose.

If a later version adds a feature that could send anything anywhere, this
statement is revised first.
