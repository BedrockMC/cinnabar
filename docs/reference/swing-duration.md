# Swing duration

## Vanilla rules

| Rule | Bedrock | Java 1.7.10 |
| --- | --- | --- |
| Base | Six simulation ticks (0.3 seconds) | Six simulation ticks (0.3 seconds) |
| Haste | Subtract amplifier + 1 | Subtract amplifier + 1 |
| Conduit Power | Use the greater Haste/Conduit level | Does not modify swings |
| Mining Fatigue | Add twice (amplifier + 1) when no speed effect is active | Add twice (amplifier + 1) when Haste is absent |
| Effect changes | Recompute duration each tick without resetting the counter | Recompute duration each tick without resetting the counter |
| Start | Counter starts at -1, then advances once per simulation tick | Counter starts at -1, then advances once per simulation tick |
| Repeat | Read the pre-increment counter and accept at half the current duration | Read the pre-increment counter and accept at half the current duration |
| Publication | Counter divided by current duration; return to zero at completion | Counter divided by current duration; return to zero at completion |
| Frames | Interpolate tick progress forward across the final wrap to rest, using the local simulation fraction | Interpolate tick progress forward across the final wrap to rest, using the local simulation fraction |

The client's local effect timeline expires finite effects on committed simulation ticks and admits only effects addressed to the local player in the current dimension. Malformed amplifiers use saturating arithmetic, with a minimum duration of one tick.

Held attempts repeat every four, three and five ticks for durations six, four and eight. A tick attempts the swing before effects expire, then publishes progress after expiry. Catch-up ticks retain both effect phases, and publication retains the final two simulation samples independently of the actor presentation clock. Bedrock packet admission and the selected animation mode use independent counters.

A pressed block swing uses the first eligible tick committed in the current frame. Held mining then continues through the remaining committed ticks in order; the same first-tick attempt is admitted only once.

Authored attack weights, pre-animation variables, arm channels and held-item channels read the same sampled local swing progress. Frame sampling leaves completed animation state and clip clocks unchanged.

Native player and held-item animation sample the interpolated attack progress. First-person attack weights remain active while the final wrap has nonzero progress; frame sampling preserves committed controller state and clip clocks.
