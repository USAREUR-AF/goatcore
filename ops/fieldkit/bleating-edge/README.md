# Bleating Edge

The design system for Fieldkit tools: small offline pages that let an operator who is not a developer do one job the
`goat` CLI does. Design: [`docs/design/fieldkit-and-bleating-edge.md`](../../../docs/design/fieldkit-and-bleating-edge.md) §4.

| File | What |
|---|---|
| `bleating-edge.css` | Tokens and the eight components |
| `bleating-edge.js` | `window.BE`: the one way a tool calls its API, and the component builders |
| `starter.html` | A working tool with one action; copy it to start a new tool |

## Tokens

| Token | Value |
|---|---|
| Palette | Sunlight mode (default: black on white, saturated signal colours) and dark mode. Dark follows the system setting; `data-theme="dark"` or `"sunlight"` on `<html>` overrides it |
| Type scale | One scale on an 18 px base: `--be-size-small`, `-base`, `-large`, `-title` |
| Targets | Every button, input and picker choice is at least 48 px tall (`--be-target`); never below 44 px |
| Focus | A 4 px orange outline on every focusable element |

## Components

| # | Component | Class | Builder |
|---|---|---|---|
| 1 | Connected or cut-off banner | `.be-banner[data-state=connected\|cutoff\|unknown]` | `BE.banner(el, status)` |
| 2 | List row for a source or rule | `.be-list` > `.be-row`, `.be-badge[data-tone=ok\|warn\|bad]` | `BE.row({title, meta, badge, button})` |
| 3 | Action button with a result preview | `.be-actions` buttons, `.be-preview` | — |
| 4 | Duration picker (4, 24, 72 hours) | `.be-duration` | `BE.duration(el, name, maxHours)` → `.hours()` |
| 5 | Confirm sheet | `dialog.be-sheet` | `BE.confirm({title, preview, verb, words, yes})` → Promise of true or false |
| 6 | "Same as" CLI strip | `.be-cli` | `BE.cli(el, verb, words)` |
| 7 | Waiting list | `.be-list.be-waiting` | `BE.row(...)` |
| 8 | Error with what-to-do text | `.be-error` | `BE.error(el, err)` (what happened, why, what to do, code) |

`BE.call(name, params, body)` calls the action declared with `data-action="name"`, filling `{param}` in its `data-api`
route. It is the only network call a tool makes; an API error body `{"error": {"code", "what", "why", "fix"}}` becomes
the error component's text.

## Delivery

A tool is one HTML file. `python3 ops/fieldkit/inline.py` copies `bleating-edge.css` and `bleating-edge.js` into every
tool between its `<style data-bleating-edge>` and `<script data-bleating-edge>` markers; the committed tool carries the
copy, so serving it needs no build. CI runs `inline.py --check`.

## Writing for operators

Operators read these pages in a second language, often outdoors. Plain words, short labels, one idea per sentence. Say
what will happen before it happens. Name the thing (the slot, the rule, the time), never "item" or "entity".
