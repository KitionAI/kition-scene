# Motor observations

Source: Blender 5.2.3 LTS, application hash `e13b728c6880`, observed on macOS arm64.
No Blender or Bullet implementation source was consulted. The public
[Motor manual](https://docs.blender.org/manual/en/3.0/physics/rigid_body/constraints/types/motor.html)
identifies the X axis and separate drive toggles; all numeric facts below come from running
`motors8.py` through the application and recording evaluated world transforms.

```sh
"$KITION_BLENDER" -b --factory-startup --python crates/engine/physics/tests/blender/motors8.py -- out.json
```

The committed output is `../fixtures/motors8-reference.json`. The Rust integration test
reads this fixture without Blender. Setting `KITION_BLENDER` additionally regenerates and
compares every recorded field in a temporary file (absolute difference below `1e-5`).

- Both drives default disabled; each target velocity and maximum impulse defaults to 1.
- The axes come from the constraint frame, regardless of the bodies' initial rotation.
  A positive target moves/spins body 1 positively relative to body 2 on/about X.
- A maximum impulse of 1 on a unit-mass body accelerates translation by 1 m/s each second.
  The bound per substep is therefore the control value times the substep duration; the
  one-step and ten-step observations distinguish this from a constant impulse per step.
- A cube of side 1 has angular inertia `mass / 6` about a face normal. A maximum impulse
  control of 1 initially accelerates a unit-mass cube by 6 rad/s each second until the
  target speed is reached. Both positive and negative targets were observed.
- Linear impulses act at the pivot, producing angular motion for offset pivots. Other axes
  remain free; an unpowered motor lets a body fall under gravity.
- Two active bodies receive opposing impulses. The unequal 2 kg and 3 kg cases record
  linear and angular momentum conservation. Solver iteration counts are also checked by
  direct solver tests against analytic impulse/mass and torque/inertia calculations.
- Disabled constraints and zero caps impart no motion. Both drives can run together.
- A motor attached to a passive body does not wake a body that starts deactivated.
  Low target speeds (0.1 m/s and rad/s) allow deactivation after about two seconds.
- A breakable motor clamps each drive impulse at the threshold and breaks when either
  reaches it, including equality. Two impulses individually below the threshold do not
  break it just because their sum exceeds the threshold. Zero thresholds produce no motion.

The 29 recorded scenes cover frames 1, 2, 3, 7, 13 and 25; the slow deactivation scene also
records frame 73. The engine comparison permits at most 2 mm position error and 0.003 rad
orientation error, allowing differences in quaternion integration. These fixtures establish
behavior for the measured cases, not general contact or arbitrary-scene parity. The verified
run measured a maximum position error of 0.000014186 m and orientation error of
0.000976563 rad across these samples.
