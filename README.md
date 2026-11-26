# Murmur — Headphone Control for Mac

**Control your Bluetooth headphones right from macOS — no sign-in, no internet, and no bulky background software.**

Normally, headphone apps only exist on your phone, or force you to create an account just to change Noise Cancelling or tweak your EQ. **Murmur** is a lightweight native macOS app that puts your headphones' real controls straight on your desktop and in your Menu Bar.

### [⬇️ Download Murmur for macOS (`Murmur-macOS.dmg`)](https://github.com/Aussie-App-Studio/Murmur-Headphone-Control-Mac/releases/latest/download/Murmur-macOS.dmg)

*macOS 12+ · Universal (Apple Silicon & Intel) · Drag to Applications · [View all releases](https://github.com/Aussie-App-Studio/Murmur-Headphone-Control-Mac/releases/latest)*  
*(If macOS says the developer cannot be verified because this is an indie GitHub release, open **System Settings → Privacy & Security** and click **Open Anyway** next to Murmur — or right-click **Murmur.app → Open** — and click **Allow** when prompted for Bluetooth access.)*

## What You Can Do
- **Noise Cancelling, Adaptive Audio & Transparency:** Switch modes or fine-tune how much outside sound comes in, with one-click voice focus and quick presets.
- **Equalizer & Bass:** Pick from built-in sound presets or drag the sliders yourself to save your own sound profiles.
- **Real Battery & Time Left:** See accurate 1%-step battery percentages (including Left, Right, and Charging Case for earbuds) plus an estimate of how many hours are left.
- **Quick One-Tap Switches:** Toggle **Speak-to-Chat** / **Conversation Awareness**, **Wearing / Ear Detection** (auto-pause), **Touch Controls**, **Multipoint**, and **DSEE** upscaling without touching your phone.
- **Change Settings Even When Offline:** Edit your settings anytime — Murmur saves them and applies them automatically next time your headphones connect.
- **Sits Quietly in Your Menu Bar:** Close the window to keep Murmur in your macOS Menu Bar (using **0% CPU** in the background) so you can switch Noise Cancelling on a call in one click.
- **Dark & Light Modes:** Automatically matches macOS, with custom color finishes for the interactive 3D headphone view.

## Which Headphones Work?
- **Full Control (Noise Cancelling, EQ, Battery & Smart Features):**
  - **Sony:** WH-1000XM6, WH-1000XM5, WH-1000XM4, WH-1000XM3, WH-1000XM2, WF-1000XM5, WF-1000XM4, WF-1000XM3, ULT WEAR, WH-CH720N, LinkBuds S, and more.
  - **Bose:** QuietComfort 45, QuietComfort Ultra, NC 700, QC35 II, QuietComfort Earbuds.
  - **Anker Soundcore:** Life Q30, Q35, Q45, Space One, Liberty 4, and more.
  - **Samsung Galaxy Buds:** Buds2, Buds2 Pro, Buds Pro, Buds Live, Buds FE, Buds3 Pro.
- **Apple AirPods & Beats (Classic L2CAP AAP + macOS Battery Fallback):**
  - **AirPods (1/2/3/4), AirPods Pro (1/2/3), AirPods Max, Powerbeats Pro, Beats Studio Pro, Beats Fit Pro, Beats Studio Buds/+, Beats Solo 4.**
  - Speaks Apple Accessory Protocol (AAP) directly over Classic L2CAP (`PSM 0x1001`) for **Noise Cancelling**, **Adaptive Audio**, **Transparency**, **Conversation Awareness**, **Automatic Ear Detection** (with live Left/Right placement), and **1%-step battery**, with automatic fallback to native macOS Bluetooth battery telemetry if L2CAP is unavailable.

## How to Use
1. Pair and connect your headphones to macOS as usual (**System Settings → Bluetooth**).
2. Open **Murmur** from Applications — it finds and connects to your headphones automatically.
3. Have **Murmur on Android, Windows, or Linux** too? Use **Save Vault / Export** to copy your exact favorite settings between devices with one JSON file.

---

## Technical Details & Developer Reference

<details>
<summary>Protocol Support, Architecture & Building from Source</summary>

### Architecture
- **Language & UI:** Pure Rust (`eframe` / `egui 0.31`, OpenGL via `glow 0.16` / `egui_glow`, `tray-icon 0.19` Menu Bar status item). Zero Electron, zero WebView runtime, optimized release binary (`lto = true`, `opt-level = "s"`, `strip = true`, `panic = "abort"`).
- **3D Hardware Renderer:** Procedural over-ear (`WH-1000XM`) and dual-earbud (`WF-1000XM`) meshes lit with Cook-Torrance GGX PBR shaders and ACES tone mapping (`#version 330 core`), paced at 30 FPS passive / 60 FPS interactive drag and paused when minimized, unfocused, or in the Menu Bar (`0.0%` hidden CPU).
- **Native macOS Bluetooth Stack (`IOBluetooth` Objective-C Bridge):**
  - Dedicated `IOBluetooth` `NSRunLoop` thread (`src/platform/macos_bt_bridge.m`) providing thread-safe RFCOMM (with SDP service record & channel resolution) and Classic L2CAP (`PSM 0x1001`) transports.
  - Supports **Sony MDR V2** (Table 1 & 2) & **MDR V1**, **Bose BMAP**, **Anker Soundcore**, **Samsung Galaxy Buds**, and **Apple AirPods AAP**.
  - Automatic AirPods/Beats battery fallback via `IOBluetoothDevice` KVC and `system_profiler SPBluetoothDataType -json`.
  - Local offline queue (`PendingOfflineChanges`), EWMA battery drain calibration, and `SettingsVaultSnapshot` camelCase JSON parity with **Murmur for Android / PC / Linux** stored in `~/Library/Application Support/murmur`.

### Build on macOS
Requires macOS 12+, Xcode Command Line Tools (`xcode-select --install`), Rust (`stable`), and optionally `librsvg` (`brew install librsvg`) for `.icns` icon generation:

```bash
cargo test
./install.sh
```

Build a Universal (`aarch64-apple-darwin` + `x86_64-apple-darwin`) drag-to-Applications `.dmg`:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
./scripts/build-dmg.sh --universal
```

</details>
