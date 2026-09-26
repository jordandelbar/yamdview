# Showcase

Run `cargo run -- examples/showcase.md` (or `yamdview examples/showcase.md`) to
see what yamdview draws.

## Text

Plain text with **bold**, _italic_, `inline code` and a
[link](https://github.com/jordandelbar/yamdview).

- A bullet list
- with two items

1. A numbered list
2. with two items

- [ ] A check
- [x] list

> A quote.

> [!NOTE]
> An alert, colored by the `info` theme role.

> [!WARNING]
> Another one, colored by `warning`.

| Column | Another column |
| ------ | -------------- |
| a      | b              |
| c      | d              |

---

## Code

```rust
fn main() {
    let name = "yamdview";
    println!("hello from {name}");
}
```

```sh
yamdview --no-mouse examples/showcase.md
```

## Flowchart

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

## Sequence diagram

```mermaid
sequenceDiagram
    participant U as User
    participant Y as yamdview
    participant T as Terminal
    U->>Y: yamdview examples/showcase.md
    Y->>Y: split markdown and diagrams
    Y->>T: styled text
    Y->>T: PNG via kitty graphics
    Note over Y,T: redrawn on save and resize
    T-->>U: document on screen
```

## Class diagram

```mermaid
classDiagram
    class Viewer {
        +PathBuf path
        +u32 scroll
        +rebuild()
        +draw()
    }
    class Search {
        +String query
        +jump()
    }
    Viewer --> Search
```

## State diagram

```mermaid
stateDiagram-v2
    [*] --> Viewing
    Viewing --> Searching: /
    Searching --> Viewing: Enter
    Searching --> Viewing: Esc
    Viewing --> [*]: q
```

## Entity relationship diagram

```mermaid
erDiagram
    DOCUMENT ||--o{ BLOCK : contains
    BLOCK ||--o| DIAGRAM : "may be"
```

## Pie chart

```mermaid
pie title Time spent
    "Reading" : 60
    "Searching" : 25
    "Scrolling" : 15
```

## Gantt chart

```mermaid
gantt
    title Release
    dateFormat YYYY-MM-DD
    section Build
    Code     :a1, 2026-09-01, 10d
    Test     :after a1, 5d
    section Ship
    Release  :2026-09-20, 2d
```
