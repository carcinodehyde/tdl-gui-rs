# tdl-gui-rs

A small desktop GUI for [tdl](https://github.com/iyear/tdl), the Telegram downloader. Paste message links, pick a folder, click **Download**.

Built with Rust and [egui](https://github.com/emilk/egui). The app does not talk to Telegram itself: it runs the `tdl` command-line tool and shows its progress.

## Features

- Large text area for Telegram message links, one per line
- **Import TXT…** to load links from a text file
- Download folder selector
- Progress bar, status line and a collapsible `tdl` output log
- **Cancel** button while a download is running

## Requirements

1. **tdl, installed and available in `PATH`**
2. **A logged-in tdl session** (done once, in a terminal)
3. **Rust**, only if you build from source

### 1. Install tdl

Pick one:

- **Windows (winget):**

  ```powershell
  winget install iyear.tdl
  ```

- **Windows (Scoop):**

  ```powershell
  scoop bucket add extras
  scoop install telegram-downloader
  ```

- **macOS / Linux (Homebrew):**

  ```sh
  brew install telegram-downloader
  ```

- **Manual:** download the archive for your system from the [tdl releases page](https://github.com/iyear/tdl/releases), extract it, and put the folder containing `tdl` (`tdl.exe` on Windows) in your `PATH`.

See the [tdl installation guide](https://docs.iyear.me/tdl/getting-started/installation/) for other options.

#### Adding tdl to `PATH` manually (Windows)

1. Extract `tdl.exe` to a permanent folder, for example `C:\Tools\tdl`.
2. Press <kbd>Win</kbd>, search for **Edit environment variables for your account**, and open it.
3. Select **Path** → **Edit** → **New**, and enter the folder path (`C:\Tools\tdl`).
4. Confirm with **OK**, then open a new terminal.

#### Verify

Open a **new** terminal and run:

```sh
tdl version
```

If this prints a version, tdl-gui-rs can find it. If you get "command not found" or "not recognized", `PATH` is not set correctly. Restart tdl-gui-rs after changing `PATH`.

### 2. Log in to Telegram with tdl

tdl-gui-rs cannot show tdl's interactive login, so log in once from a terminal:

```sh
tdl login -T qr
```

Scan the QR code with the Telegram mobile app (**Settings → Devices → Link Desktop Device**). To log in with a phone number and code instead:

```sh
tdl login -T code
```

The session is stored by tdl and reused by the GUI. See the [tdl login guide](https://docs.iyear.me/tdl/getting-started/quick-start/) for more options.

## Build and run

Install Rust from [rustup.rs](https://rustup.rs/) (on Windows, `winget install Rustlang.Rustup`; the MSVC toolchain also needs the Visual Studio C++ Build Tools).

```sh
git clone <this repository>
cd tdl-gui
cargo run --release
```

Or build once and run the executable directly:

```sh
cargo build --release
```

The binary is written to `target/release/tdl-gui-rs` (`tdl-gui-rs.exe` on Windows). It is self-contained and can be copied anywhere.

On Linux, egui and the file dialogs need a few system packages. On Debian/Ubuntu:

```sh
sudo apt install libgtk-3-dev libxkbcommon-dev libwayland-dev libxcb1-dev
```

## Usage

1. Paste Telegram message links into the text area, one per line, or click **Import TXT…** to load them from a file.
2. Choose the download folder.
3. Click **Download**.

Link format:

```text
https://t.me/channel_name/123
https://t.me/c/1234567890/456
```

Blank lines and lines starting with `#` are ignored, and duplicate links are downloaded once.

To get a link in Telegram, right-click a message and choose **Copy Message Link**.

## How it works

For a download, the app runs:

```sh
tdl dl -d <folder> --continue -u <link> -u <link> ...
```

`--continue` makes tdl resume an interrupted download of the same links instead of asking on the console.

## Troubleshooting

| Problem | Fix |
| --- | --- |
| "`tdl` was not found in PATH" | Install tdl, check `tdl version` in a new terminal, then restart the app. |
| tdl exits with an error about authorization or login | Run `tdl login -T qr` in a terminal. |
| Download fails or finishes instantly | Expand **tdl output** in the app to see what tdl reported. |
| Progress bar stays at 0% while files are arriving | The bar is parsed from tdl's text output, which can change between tdl versions. The download itself is unaffected. |
