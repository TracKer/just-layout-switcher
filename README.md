# just-layout-switcher
Just layout switcher

A Windows tray app. Right-click the tray icon for **Settings**, **About** and **Exit**
(double-click opens Settings).

The hotkey is stored in `config.yaml` next to the executable (created on first start):

```yaml
hotkey: Alt + Shift
```

To change it, open Settings → General, click the field and press the new combination.
It is saved as soon as any of the pressed keys is released.

## Build

```
cargo build --release
```
