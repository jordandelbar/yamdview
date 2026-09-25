# yamdview

Render markdown in the terminal, with mermaid diagrams drawn as images
(kitty graphics protocol: Ghostty, kitty, WezTerm). No browser involved:
diagrams go through [merman](https://github.com/Latias94/merman).

```sh
cargo run --release -- README.md
```

A live viewer: it re-renders when the file is saved or the pane is resized,
so run it in a split next to your editor. Diagrams pick up Ghostty's colors
and font (`ghostty +show-config`).

Keys: `j`/`k` or arrows, `space`/`b` page, `ctrl-d`/`ctrl-u` half page,
`g`/`G` top/bottom, mouse wheel, `q` to quit.

## How it works

```mermaid
flowchart LR
    A[README.md] --> B{pulldown-cmark}
    B -->|text| C[tui-markdown]
    B -->|mermaid block| D[merman]
    D --> E[PNG]
    E --> F[kitty protocol]
    C --> G((terminal))
    F --> G
```

## A sequence diagram

```mermaid
sequenceDiagram
    participant U as User
    participant M as yamdview
    participant T as Ghostty
    U->>M: yamdview README.md
    M->>M: split markdown
    M->>T: styled text
    M->>T: PNG via ESC _G
    T-->>U: diagram on screen
```

Other code blocks stay as code:

```rust
fn main() { println!("hi"); }
```
