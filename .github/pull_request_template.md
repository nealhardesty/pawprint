## What and why

<!-- What this changes and why. Link ROADMAP items it checks off. -->

## Checks

- [ ] `make ci` passes locally
- [ ] SPEC.md, README.md, and ROADMAP.md are updated where facts changed

## Art review (only for PRs that add or change mascots)

Fill this in for **every** mascot touched (SPEC §12.6). Attach the relevant
crop of `docs/gallery.png` (`make contact-sheet`) and, for actions,
`docs/gallery.gif` (`make contact-sheet ANIMATED=1`).

### `<mascot-id>`

- [ ] **Squint test:** recognizable at 1× (32 px) with the label covered
- [ ] Distinct from every sibling on the same base
- [ ] Species-defining feature exaggerated (see the art bible, SPEC §12.10)
- [ ] Face and silhouette in rows 0–19
- [ ] Each action reads in under a second and returns cleanly to rest
- [ ] Signature action matches its Appendix A brief
- [ ] `make check-assets` is clean, with no warnings
- [ ] Original art: nothing traced from or derived from existing games, characters, or other copyrighted art
