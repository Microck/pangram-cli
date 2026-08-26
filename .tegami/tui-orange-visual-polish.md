---
packages:
  "cargo:microck-pangram-cli": patch
  "npm:@microck/pangram-cli": patch
---

## Changed

Refined the TUI with Pangram orange as its primary color, layered charcoal
navigation and composer surfaces, a dark neutral canvas in both truecolor and
ANSI terminals, a bottom-aligned Analyze dock on standard terminals, one
orange primary action, visible selector and toggle targets, a consistent
vertical rhythm between controls and sections, padded modal content, clearer
focus and selection cues, symmetric button padding, and mouse support for
routes, controls, lists, results, and the command bar. Focus and selection now
use orange, weight, fill, or markers without terminal underlining anywhere.
The selected route uses orange fill with dark text from the first frame, before
the route rail receives keyboard or mouse focus.
Inactive routes now use evenly padded charcoal targets in both responsive
layouts. On wide terminals, command-bar shortcuts now align with the center
workspace instead of extending beneath the route rail, and the route surface
and separator continue cleanly through the bottom row.
The approved Pangram fox now plays as a terminal-native 2.8-second sequence at
20 frames per second, with orange-dominant truecolor, ANSI, and `NO_COLOR`
rendering, a responsive size floor, skip controls, and one-time playback state.
Its backdrop now settles into the TUI canvas while the fox runs, then the real
Analyze screen fades in over 300 ms instead of appearing in a hard cut.
