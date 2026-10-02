# Changelog

## [0.5.0](https://github.com/jordandelbar/yamdview/compare/v0.4.1...v0.5.0) (2026-10-02)


### Features

* Show links as clickable text without their URL ([#28](https://github.com/jordandelbar/yamdview/issues/28)) ([de29677](https://github.com/jordandelbar/yamdview/commit/de29677203a9491f0b1d45255bc4a89656b68ad3))


### Refactor

* Parse documents into a format-neutral model ([#25](https://github.com/jordandelbar/yamdview/issues/25)) ([0aa0934](https://github.com/jordandelbar/yamdview/commit/0aa0934812ba744bb36ed9c09113f7bfc4df0def))

## [0.4.1](https://github.com/jordandelbar/yamdview/compare/v0.4.0...v0.4.1) (2026-10-01)


### Refactor

* Extract argument parsing and enforce rustfmt ([#21](https://github.com/jordandelbar/yamdview/issues/21)) ([cd85a53](https://github.com/jordandelbar/yamdview/commit/cd85a538b17d7e57e36ba0179ac9f44aea1a3e49))
* Simplify Viewer:rebuild and share viewer test setup ([#24](https://github.com/jordandelbar/yamdview/issues/24)) ([ecbd82e](https://github.com/jordandelbar/yamdview/commit/ecbd82e36fd03dc783516203f73e74f51ecdd69c))
* Split main.rs into cli, kitty, viewer and tui modules ([#23](https://github.com/jordandelbar/yamdview/issues/23)) ([7ae15a1](https://github.com/jordandelbar/yamdview/commit/7ae15a183c60ac9e33f0e4b11b748389b33fdeca))

## [0.4.0](https://github.com/jordandelbar/yamdview/compare/v0.3.0...v0.4.0) (2026-09-26)


### Features

* Print the rendered document with --print ([#18](https://github.com/jordandelbar/yamdview/issues/18)) ([5820159](https://github.com/jordandelbar/yamdview/commit/58201590d494c0448d8eceb07547c3011d78772c))


### Bug Fixes

* Keep tmux refresh errors off the screen in detached sessions ([#17](https://github.com/jordandelbar/yamdview/issues/17)) ([9fb2992](https://github.com/jordandelbar/yamdview/commit/9fb2992ef6a2e19a3260f59476187dc6a347147b))

## [0.3.0](https://github.com/jordandelbar/yamdview/compare/v0.2.0...v0.3.0) (2026-09-26)


### Features

* Better rendering of alerts and checkboxes ([#10](https://github.com/jordandelbar/yamdview/issues/10)) ([c0b21a7](https://github.com/jordandelbar/yamdview/commit/c0b21a7bc379fe04e5a80fc26ee0a78414a24003))
* Jump between headings ([#14](https://github.com/jordandelbar/yamdview/issues/14)) ([99aa01d](https://github.com/jordandelbar/yamdview/commit/99aa01d24838b5569492e80df3d2915b629bef61))
* Read markdown from stdin ([#13](https://github.com/jordandelbar/yamdview/issues/13)) ([dc5aa14](https://github.com/jordandelbar/yamdview/commit/dc5aa1409ddea1c6550732edcf0c642fdfdce164))
* Show diagram source in terminals without kitty graphics ([#16](https://github.com/jordandelbar/yamdview/issues/16)) ([2ac1cd0](https://github.com/jordandelbar/yamdview/commit/2ac1cd00bd64f47cf13fe605a94b33bd144c7768))

## [0.2.0](https://github.com/jordandelbar/yamdview/compare/v0.1.0...v0.2.0) (2026-09-26)


### Features

* Add interactive search and tmux mouse selection ([303fbf2](https://github.com/jordandelbar/yamdview/commit/303fbf255424aa15117d92fc6eb84f4691fa9ddb))
* Markdown viewer with mermaid diagrams ([c3afc80](https://github.com/jordandelbar/yamdview/commit/c3afc80352e455179718eee1f511fdf6df8162fd))
* Semantic theme roles, loadable from ~/.config/yamdview/theme ([c1ee27c](https://github.com/jordandelbar/yamdview/commit/c1ee27cc9edc4cbf5d12725732d197b65a99eb6a))


### Bug Fixes

* Keep the viewer open when tmux copy fails, and selection off the status line ([ca2ecdc](https://github.com/jordandelbar/yamdview/commit/ca2ecdcf6ef4c3e3bd353e4233004f3ef7350a20))
* Search navigation, lazy search layout ([d31e3cd](https://github.com/jordandelbar/yamdview/commit/d31e3cd3a503fc6ae73cf981c805edc76a63d73a))
