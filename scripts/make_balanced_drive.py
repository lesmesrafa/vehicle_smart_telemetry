#!/usr/bin/env python3
"""Writes a made-up drive in which every answer of the questions of questions.tsv happens often.

A real recording is mostly steady driving: in the X-CANIDS dataset 4 windows in 100 have a hard
braking and 1 in 10 a turn, so an accuracy measured there says little. Here each window of 5 s
does one thing, drawn at random: stand still, hold the speed, speed up, brake, brake hard, or
slow down without the brake; and, at low speed, turn left or right. Every value stays clear of
the limits the questions state, so that the right answer of a window is never in doubt.

The signals agree with each other as in scripts/make_drive.py, and the car resembles the one of
X-CANIDS: the brake circuit rests at 1.5 bar and the brake flag is 1 at rest and 2 in use.

    python3 scripts/make_balanced_drive.py hyundai_2015_ccan.dbc data/balanced.log
    python3 scripts/make_balanced_drive.py hyundai_2015_ccan.dbc data/balanced.log --windows 1200 --seed 2
"""
import argparse
import math
import random
import re
import sys

RATE_HZ, WINDOW_S = 20, 5
WHEELBASE_M, TRACK_M, STEERING_RATIO = 2.7, 1.6, 15.0
START = 1700000000.0
REST_BAR = 1.5


def read_dbc(path):
    """The messages of a DBC: name -> (identifier, [(signal, start bit, bits, signed, factor, offset)])."""
    messages, current = {}, None
    for line in open(path, encoding="latin-1"):
        head = re.match(r"BO_ (\d+) (\w+):", line)
        if head:
            current = messages.setdefault(head.group(2), (int(head.group(1)), []))
            continue
        signal = re.match(r"\s+SG_ (\w+) (?:\w+ )?: (\d+)\|(\d+)@([01])([+-]) \(([^,]+),([^)]+)\)", line)
        if signal and current is not None:
            name, start, bits, order, sign, factor, offset = signal.groups()
            if order != "1":
                sys.exit(f"error: {name} is not written the Intel way, which is all this script knows")
            current[1].append((name, int(start), int(bits), sign == "-", float(factor), float(offset)))
    return messages


def encode(signals, values):
    """The bytes of a message from the physical value of some of its signals; the rest stay at raw zero."""
    word = 0
    for name, start, bits, signed, factor, offset in signals:
        if name not in values:
            continue
        raw = round((values[name] - offset) / factor)
        low, high = (-(1 << (bits - 1)), (1 << (bits - 1)) - 1) if signed else (0, (1 << bits) - 1)
        word |= (max(low, min(high, raw)) & ((1 << bits) - 1)) << start
    return word.to_bytes(8, "little")


def clear_of_limits(speed):
    """Whether a speed at the end of a window leaves no doubt about its band: stopped, slow, medium or fast."""
    return speed == 0 or (speed >= 3 and abs(speed - 30) >= 2.5 and abs(speed - 80) >= 2.5 and speed <= 110)


def plan(windows, rng):
    """What the car does in each window: (speed at the start, speed at the end, kind, a number of
    the kind, steering angle at the peak). Speeds in km/h. The car is drawn towards a speed that
    changes every few windows, so that it spends time stopped, slow, at a medium speed and fast."""
    speed, aim, done = 0.0, 0.0, []
    while len(done) < windows:
        if len(done) % 8 == 0 and (not done or done[-1][1] == speed):
            aim = rng.choice((0, 0, 18, 18, 22, 50, 65, 95, 100))
        if speed == 0:
            kinds = ["stand"] if aim == 0 else ["stand", "faster", "faster"]
        elif speed < aim - 10:
            kinds = ["faster", "faster", "faster", "steady", "coast", "hard"]
        elif speed > aim + 10:
            kinds = ["brake", "brake", "coast", "coast", "hard", "hard", "steady"]
        else:
            kinds = ["steady", "steady", "faster", "brake", "coast", "hard"]
        kind, extra = rng.choice(kinds), 0.0
        if kind in ("stand", "steady"):
            end = speed
        elif kind == "faster":
            end = speed + rng.uniform(8, 30)
        elif kind == "brake":
            end = 0.0 if speed <= 28 and aim == 0 else speed - rng.uniform(8, 28)
        elif kind == "coast":
            end = speed - rng.uniform(6.5, 9)
        else:
            # A hard braking: `extra` m/s^2 for long enough to lose what is lost.
            extra = rng.uniform(2.6, 4.5)
            end = speed - extra * rng.uniform(1.0, 1.8) * 3.6
        end = round(end * 2) / 2
        if end < 0 or not clear_of_limits(end) or (kind in ("coast", "hard") and end < 5):
            continue
        # A turn, only where the car goes slowly enough for it and is not braking hard.
        angle = 0.0
        if kind != "hard" and 8 <= min(speed, end) and max(speed, end) <= 35 and rng.random() < 0.8:
            angle = rng.choice((-1, 1)) * rng.uniform(45, 90)
        done.append((speed, end, kind, extra, angle))
        speed = end
    return done


def car(done, t):
    """Speed in km/h, whether the brake is in use and the steering wheel angle at time t."""
    index = min(int(t // WINDOW_S), len(done) - 1)
    start, end, kind, extra, angle = done[index]
    into = t - index * WINDOW_S
    if kind == "hard":
        # Steady, then the braking from the second 1.5, then steady again.
        length = (start - end) / 3.6 / extra
        speed = start if into < 1.5 else end if into > 1.5 + length else start - extra * (into - 1.5) * 3.6
        braking = 1.5 <= into <= 1.5 + length
    else:
        speed = start + (end - start) * into / WINDOW_S
        braking = kind == "brake"
    # The steering wheel goes to its peak, stays and comes back, all inside the window.
    turn = 0.0 if into < 1.0 or into > 4.0 else min(1.0, (into - 1.0) / 0.75, (4.0 - into) / 0.75)
    return speed, braking, angle * turn


def state(done, t):
    """Every signal of the car at time t, by message."""
    step = 0.5 / RATE_HZ
    speed, braking, angle = car(done, t)
    accel = (car(done, t + step)[0] - car(done, max(t - step, 0.0))[0]) / 3.6 / (step + min(t, step))   # m/s^2
    yaw = speed / 3.6 / WHEELBASE_M * math.tan(math.radians(angle / STEERING_RATIO))                    # rad/s
    # Turning left, which is a positive angle, the right wheels are the outer ones and go faster.
    left, right = speed - yaw * TRACK_M / 2 * 3.6, speed + yaw * TRACK_M / 2 * 3.6
    pushing = accel > 0.05
    return {
        "CLU11": {"CF_Clu_Vanz": speed},
        "WHL_SPD11": {"WHL_SPD_FL": left, "WHL_SPD_FR": right, "WHL_SPD_RL": left, "WHL_SPD_RR": right},
        "ESP12": {
            "LONG_ACCEL": accel,
            "LAT_ACCEL": speed / 3.6 * yaw,
            "YAW_RATE": math.degrees(yaw),
            "CYL_PRES": REST_BAR + 12.0 * abs(accel) if braking else REST_BAR,
        },
        "SAS11": {"SAS_Angle": angle},
        "EMS12": {"BRAKE_ACT": 2 if braking else 1, "PV_AV_CAN": 8.0 + 15.0 * accel if pushing else 0.0},
        "EMS11": {"N": 800.0 + 35.0 * speed, "VS": speed},
        "TCU12": {"VS_TCU": math.floor(speed)},
    }


def main():
    parser = argparse.ArgumentParser(description="A made-up drive in which every answer happens often.")
    parser.add_argument("dbc", help="the DBC of the car")
    parser.add_argument("output", help="the log to write")
    parser.add_argument("--windows", type=int, default=600, help="windows of 5 s to write (default 600, which is 50 minutes)")
    parser.add_argument("--seed", type=int, default=8, help="the same seed gives the same drive (default 8)")
    args = parser.parse_args()

    dbc = read_dbc(args.dbc)
    done = plan(args.windows, random.Random(args.seed))
    frames = 0
    with open(args.output, "w") as out:
        for i in range(RATE_HZ * WINDOW_S * args.windows):
            t = i / RATE_HZ
            for k, (name, values) in enumerate(state(done, t).items()):
                identifier, signals = dbc[name]
                # The messages of one instant leave 1 ms apart, like frames sharing one bus.
                out.write(f"({START + t + 0.001 * k:.6f}) vcan0 {identifier:03X}#{encode(signals, values).hex().upper()}\n")
                frames += 1
    kinds = {}
    for _, _, kind, _, angle in done:
        kinds[kind] = kinds.get(kind, 0) + 1
    turns = sum(1 for d in done if d[4] > 0), sum(1 for d in done if d[4] < 0)
    print(f"{frames} frames, {args.windows} windows of {WINDOW_S} s, written to {args.output}")
    print("windows by what the car does: " + ", ".join(f"{k} {n}" for k, n in sorted(kinds.items()))
          + f"; turning left {turns[0]}, right {turns[1]}")


if __name__ == "__main__":
    main()
