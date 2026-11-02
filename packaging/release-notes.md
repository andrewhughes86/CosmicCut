CAM for XYB hot-wire foam cutters: import or sketch shapes, set up cuts, simulate them, and post G-code.

## Download

| System | File |
| --- | --- |
| **Windows** 10/11 (64-bit) | [Installer](https://github.com/andrewhughes86/CosmicCut/releases/download/@TAG@/CosmicCut-windows-x64-setup.exe) · [Portable zip](https://github.com/andrewhughes86/CosmicCut/releases/download/@TAG@/CosmicCut-windows-x64.zip) |
| **macOS** 11+ (Apple silicon and Intel) | [Disk image (.dmg)](https://github.com/andrewhughes86/CosmicCut/releases/download/@TAG@/CosmicCut-macos-universal.dmg) |
| **Linux** x86-64, Debian/Ubuntu | [.deb package](https://github.com/andrewhughes86/CosmicCut/releases/download/@TAG@/CosmicCut-linux-amd64.deb) |
| **Linux** x86-64, any | [tar.gz](https://github.com/andrewhughes86/CosmicCut/releases/download/@TAG@/CosmicCut-linux-x86_64.tar.gz) |
| **Browser** | [Run it on the web](https://andrewhughes86.github.io/CosmicCut/) — nothing to install |

## Installing

- **Windows:** run the installer. The app isn't code-signed yet, so SmartScreen may say
  "Windows protected your PC": choose **More info → Run anyway**.
- **macOS:** open the .dmg and drag CosmicCut to Applications. It isn't notarized yet, so the
  first time, right-click (or Control-click) CosmicCut in Applications and choose **Open**, then
  **Open** again. If macOS says the app is damaged, run
  `xattr -dr com.apple.quarantine /Applications/CosmicCut.app` in Terminal.
- **Linux (.deb):** `sudo apt install ./CosmicCut-linux-amd64.deb`, then start CosmicCut from the
  app menu or with `cosmiccut`.
- **Linux (tar.gz):** unpack it and run `./CosmicCut`; `./install.sh` adds it to your app menu
  for your user.

Settings, machines and post processors are kept per user and carry over between versions.
