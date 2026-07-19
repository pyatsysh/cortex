# Building Cortex on Windows

Everything below is run from **PowerShell** (open *Windows Terminal* or *PowerShell*
from the Start menu — no admin needed to start, though installers will pop UAC prompts).

The project is already on your Windows drive at:

    C:\Users\pyats\projects\System_Monitor

---

## Step 1 — Install the toolchain (one-time)

Run these four `winget` commands. (If `winget` isn't found, install **App Installer**
from the Microsoft Store, or use the GUI downloads noted at the bottom.)

```powershell
# 1. Rust (installs rustup + the MSVC toolchain)
winget install --id Rustlang.Rustup -e

# 2. Visual C++ build tools + Windows SDK (the linker Rust needs). ~2-4 GB.
#    Skip if you already have Visual Studio 2022 with "Desktop development with C++".
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

# 3. Node.js LTS (provides npm, used to fetch the Tauri CLI)
winget install --id OpenJS.NodeJS.LTS -e

# 4. WebView2 runtime — already present on Win10/11; run only if the app later says it's missing
winget install --id Microsoft.EdgeWebView2Runtime -e
```

> A reboot may be requested after the C++ build tools install — do it if asked.

**Close and reopen the terminal** so the new PATH takes effect, then make sure Rust
uses the MSVC toolchain:

```powershell
rustup default stable-msvc
```

## Step 2 — Verify the toolchain

```powershell
rustc --version
cargo --version
node --version
npm --version
```

All four should print a version. If `rustc`/`cargo` aren't found, reopen the terminal
(PATH didn't refresh yet).

## Step 3 — Go to the project

```powershell
cd $HOME\projects\System_Monitor
```

## Step 4 — Fetch the Tauri CLI

```powershell
npm install
```

## Step 5 — Run it

```powershell
npm run dev
```

The **first** run compiles the Rust crates — expect a few minutes and a lot of
`Compiling …` lines. Then a window titled **"Cortex — System Monitor"** opens showing
**live** data (the status dot at the bottom turns green and reads *Live*).

Edit `src\index.html` while it runs and the window hot-reloads.

## Step 6 — If the Rust build fails

This is expected-possible on the first compile (the `sysinfo` crate's API is
version-sensitive). Capture the full output and send it back:

```powershell
npm run dev 2>&1 | Tee-Object -FilePath build-log.txt
```

Then paste the contents of `build-log.txt` (or at least the first `error[...]` block).
The fixes are small and local — I'll patch `src-tauri\src\main.rs` and you re-run Step 5.

## Step 7 — Produce installers (later, optional)

```powershell
npm run build
```

Output lands in:

    src-tauri\target\release\bundle\

- `msi\`   → `Cortex_0.1.0_x64_en-US.msi`
- `nsis\`  → `Cortex_0.1.0_x64-setup.exe`

---

### What each result means

| Outcome | Meaning |
|---|---|
| Window opens, green *Live* dot, numbers move | ✅ Working. CPU/RAM/disk/net/processes are real. |
| GPU ring shows **N/A** | No NVIDIA GPU / driver — expected on non-NVIDIA machines. |
| CPU temperature shows **--** | Windows has no CPU temp sensor without a driver — expected; deferred to a later phase. |
| Build stops with `error[E...]` | Send me `build-log.txt` (Step 6). |
| `link.exe not found` / linker error | C++ build tools (Step 1.2) didn't install — rerun it, reboot. |

### GUI download fallbacks (if you skip winget)

- Rust: <https://rustup.rs>
- VS Build Tools: <https://visualstudio.microsoft.com/downloads/> → "Build Tools for Visual Studio 2022" → check **Desktop development with C++**
- Node.js LTS: <https://nodejs.org>
