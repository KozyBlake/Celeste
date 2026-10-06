# Windows testing

Celeste has native Windows code paths for its data folder, VRChat logs,
process launching, file dialogs and text-to-speech. Linux testing cannot prove
those paths work, so the repository includes a native `windows-latest` GitHub
Actions job.

The release executable embeds the Celeste icon and uses the Windows GUI
subsystem, so launching it normally does not leave an extra console window.
Diagnostic output is written to Celeste's log folder instead.
MSVC builds statically link the C runtime, so the downloaded executable does
not require users to install the Visual C++ redistributable separately.

## Automated Windows build

The **Rust Windows** workflow runs the complete workspace test suite, creates
an optimized Windows build and uploads `celeste-windows-x64` as a downloadable
artifact. It runs for `main`, pull requests and manual **Run workflow** jobs.

The workflow catches Windows compiler, path, dependency and unit-test failures.
It cannot sign into real Discord or VRChat accounts, inspect the VRChat client,
play audio hardware, or visually inspect the desktop window.

## Hands-on smoke test

Download `celeste-windows-x64`, extract `celeste-testing.exe`, and keep Java
Scarlet closed while testing migration. Back up the Scarlet data folder first.

For an isolated first run in PowerShell:

```powershell
$env:CELESTE_HOME = "$env:TEMP\CelesteSmokeTest"
.\celeste-testing.exe
```

For a real migration test, remove that environment override and launch Celeste
as a user with one of these existing folders:

- `%LOCALAPPDATA%\SybylineNetwork\Scarlet`
- `%LOCALAPPDATA%\KozyBlake\Scarlet`
- `%USERPROFILE%\AppData\Local\SybylineNetwork\Scarlet`
- `%USERPROFILE%\AppData\Local\KozyBlake\Scarlet`

Verify the following before calling the Windows build releasable:

1. Celeste detects the legacy folder, offers the correct path, and copies its
   durable data without changing the Scarlet source.
2. Cancelling detection starts clean; choosing **No** allows a different
   Scarlet folder to be selected manually.
3. Setup accepts Discord and VRChat credentials, including both authenticator
   and email two-factor flows, then restores the session after restart.
4. VRChat's newest `output_log_*.txt` under
   `%USERPROFILE%\AppData\LocalLow\VRChat\VRChat` updates the instance roster.
5. Discord commands, moderation threads, tags, voice connection and TTS work
   against a test server.
6. File import dialogs, data-folder opening/moving, VRChat launch/relaunch,
   window resizing, icon/taskbar identity and settings persistence behave
   normally.
7. Closing and restarting Celeste preserves settings and produces useful logs
   in the selected data folder.

If something fails, preserve the Celeste log and the exact Windows version,
whether VRChat came from Steam, and whether PowerShell or `pwsh` is installed.
