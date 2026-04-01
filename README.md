# Px to Rem

Converts CSS `px` values to `rem` and vice versa directly in the editor.

![Px to Rem](cover.png)

## Install

Install the extension in Zed.

The extension will use `px-to-rem-lsp` from your `PATH` if it already exists. Otherwise it downloads the correct binary from the latest GitHub release automatically.

## Usage

Place the cursor on a `px` or `rem` value and open code actions with `cmd+shift+z`, then choose a conversion.

If you select a range first, the extension also offers batch conversions for all matches in the selection.

## Configuration

Add settings in `~/.config/zed/settings.json`:

```json
{
  "lsp": {
    "px-to-rem-lsp": {
      "initialization_options": {
        "px_per_rem": 16,
        "decimal_places": 4
      }
    }
  }
}
```

| Option | Default | Description |
|---|---|---|
| `px_per_rem` | `16` | How many pixels equal `1rem` |
| `decimal_places` | `4` | Max decimal places in the result |

## Supported File Types

CSS, SCSS, Less, HTML, JavaScript, TypeScript

## Build

```sh
cargo build -p px-to-rem-lsp --release
cargo build --lib --target wasm32-wasip1 --release
```
