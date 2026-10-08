# Reviewed v1 examples

`reviewed-v1.json` contains two generated wire positions per family from master
seed 20261008, their attempt seeds/parameters and expected direction labels.
The generation acceptance tests reconstruct the constructors and independently
solve/verify these examples; stored labels are assertions, not oracle input.

Review observations (coordinates use y upwards):

- Unique escape: corner heads at (10,10) and (0,10), own neck blocks the other
  inward exit. Left and down respectively are the only surviving root moves.
- Head contest: heads approach the initial food. The sideways alternate food
  route survives the three-turn health deadline; the sampled contest routes do
  not robustly survive every simultaneous response. The rules tests separately
  check equal and unequal food-contest growth.
- Tail/growth: the length-four example permits entry into its vacating tail;
  the length-five example has a duplicate tail and rejects that entry. Movement
  then growth creates a duplicate *new* tail, as checked by reference tests.
- Food access: the four-health example has foods at (5,5) and (8,7), from a head
  at (8,5). Up is blocked by the neck. Left reaches the farther food; right can
  detour around the body to the nearer food. The two-health example must take
  the adjacent food at (3,7); a direct Manhattan direction alone is insufficient.
- Delayed trap: head (5,4), length 20. Right enters the folded body corridor;
  its delaying continuation reaches self-collision after five transitions.
  Left is the certified escape. The walls are moving bodies, not frozen obstacles.
- Forced terminal win: health two for focal and rival, food two steps above
  focal. Only up gets food before starving while the isolated rival's health
  runs out. This is a true finite sole-survivor objective, with all alternatives
  proved failure, but a deliberately restricted endgame construction.

All examples are synthetic and have no future spawning or hazards. Their small
horizons/controlled geometry are a tractability choice, not a claim of live-game
frequency or general tactical difficulty.
