# Using yamdview

```sh
yamdview [--mouse | --no-mouse] [--] [FILE | -]
```

`FILE` defaults to `README.md`. yamdview redraws when the file is saved and
when the terminal is resized.

It also reads markdown from stdin, with `-` or when you pipe something in
without naming a file:

```sh
gh pr view 12 | yamdview
curl -s https://raw.githubusercontent.com/jordandelbar/yamdview/main/README.md | yamdview
```

| Option       | Effect                                                           |
| ------------ | ---------------------------------------------------------------- |
| `--mouse`    | Capture the mouse. This is the default.                          |
| `--no-mouse` | Leave the mouse to the terminal, for its own selection behavior. |
| `--`         | Treat the next argument as the file, even if it starts with `-`. |

## Keys

| Key                 | Action                                       |
| ------------------- | -------------------------------------------- |
| `j`, `Down`         | Scroll down one line                         |
| `k`, `Up`           | Scroll up one line                           |
| `space`, `PageDown` | Page down                                    |
| `b`, `PageUp`       | Page up                                      |
| `ctrl-d`, `ctrl-u`  | Half a page down, up                         |
| `g`, `Home`         | Go to the top                                |
| `G`, `End`          | Go to the bottom                             |
| `]`, `[`            | Next, previous heading                       |
| `/`                 | Start a search                               |
| `n`, `N`            | Next, previous match                         |
| `Esc`               | Clear the search, or quit if there isn't one |
| `q`, `ctrl-c`       | Quit                                         |
| Mouse wheel         | Scroll three lines                           |

## Search

Press `/` and type. The view jumps to the first match at or below where you
started, and every match is highlighted. The bottom line shows the query and a
counter such as `[2/5]`.

| Key while typing | Action                                        |
| ---------------- | --------------------------------------------- |
| `Enter`          | Keep the search and go back to scrolling      |
| `Esc`            | Cancel and return to where the search started |
| `Backspace`      | Delete the last character                     |

`n` and `N` wrap around at the ends of the document. If you scroll, or the file
reloads, they continue from the top of the screen instead of the last match.

Search is literal and case-sensitive, and it works one displayed line at a
time. A phrase that wraps onto the next line won't match, and text inside
diagrams can't be searched.

## Mouse and tmux

Outside tmux, the wheel scrolls the document. To select text, drag while
holding your terminal's selection modifier, usually Shift.

Inside tmux, yamdview handles selection itself. Drag with the left button to
highlight text; releasing copies it into tmux's paste buffer, and `prefix ]`
pastes it. yamdview also asks tmux to pass the text on to the system
clipboard, which works if your tmux `set-clipboard` setting and your terminal
allow it. Diagram placeholders are left out of copied text. Scrolling, typing,
resizing or a reload clears the selection.

tmux needs two settings in `~/.tmux.conf`:

```sh
set -g mouse on              # send mouse events to yamdview
set -g allow-passthrough on  # let diagram images through to the terminal
```

See the tmux wiki on
[mouse support](https://github.com/tmux/tmux/wiki/Getting-Started#using-the-mouse).

## Theme

Colors come from the first of these that exists:

1. `~/.config/yamdview/theme`
2. Ghostty's palette (`ghostty +show-config`)
3. Dracula

The theme file has one `role = #rrggbb` line per role, and every role is
required. An optional `font-family = Name` line sets the font used in
diagrams.

| Roles                                                                     | Used for                                                                               |
| ------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `background`, `foreground`                                                | Page background and body text                                                          |
| `selection`                                                               | Search matches and mouse selection                                                     |
| `muted`                                                                   | Rules, table borders, metadata                                                         |
| `heading`, `bold`, `italic`, `code`, `link`, `quote`                      | Markdown text (`code` is inline code)                                                  |
| `comment`, `keyword`, `string`, `function`, `type`, `number`, `parameter` | Syntax highlighting in code blocks                                                     |
| `accent`                                                                  | Diagram borders and node tint, the current match, `[!IMPORTANT]` alerts                |
| `note`                                                                    | Diagram notes                                                                          |
| `info`, `success`, `warning`, `error`                                     | The other alerts (`[!NOTE]`, `[!TIP]`, `[!WARNING]`, `[!CAUTION]`), and diagram colors |
