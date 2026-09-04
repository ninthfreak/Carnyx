# Drive logs off the unit

Exports of the in-app diagnostics log, as the owner sent them. The head unit has
no adb, so this is the ONLY channel a fault on the device can reach a developer
through — and a log that is read once, for one question, and then discarded takes
every other answer in it with it.

That has happened. `2026-09-03-drive.txt` was mined for the four interface
defects of #126 and the probe sections in it — which answer the whole of #133 —
went unread for a month. It was then reported as unrecoverable, having never been
saved anywhere. It lives here now, and so does every one after it.

Name them `YYYY-MM-DD-drive.txt`. Say in the TASKS entry which log a finding came
from, and quote the line.
