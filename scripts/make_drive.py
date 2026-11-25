#!/usr/bin/env python3
"""Writes sample/drive.log: 21 s of a made-up drive whose signals agree with each other.

It is how the sample was made; nothing else in the project runs it. It needs cantools
(pip install cantools) and the DBC.

    python3 scripts/make_drive.py hyundai_2015_ccan.dbc sample/drive.log

The drive: it speeds up from 30 to 60 km/h (0 to 4 s), holds 60 (4 to 6 s), brakes hard down
to 15 km/h (6 to 8.5 s), speeds up to 30 (8.5 to 11 s), turns left at 30 km/h (11 to 14 s),
speeds up to 50 (15 to 19 s) and holds it until the end. Nine messages, 50 frames per second
each. No randomness: the same file comes out every time.

Besides the signals the graph builder names, it fills in four that only have a default quantity:
the copies of the speed that the engine and the transmission send, and the lateral position of
the vehicle ahead.
"""
import math
import sys

import cantools

RATE_HZ = 50
SECONDS = 21
WHEELBASE_M, TRACK_M, STEERING_RATIO = 2.7, 1.6, 15.0
START = 1700000000.0

# (time in s, speed in km/h): the speed is a straight line between each pair of points.
SPEED = [(0, 30), (4, 60), (6, 60), (8.5, 15), (11, 30), (15, 30), (19, 50), (SECONDS, 50)]
# (time in s, steering wheel angle in degrees), positive to the left.
STEERING = [(0, 0), (11, 0), (12, 120), (13, 120), (14, 0), (SECONDS, 0)]


def along(points, t):
    """Value at time t of a line drawn through the points. Before the first point, its value."""
    if t <= points[0][0]:
        return points[0][1]
    for (t0, v0), (t1, v1) in zip(points, points[1:]):
        if t0 <= t <= t1:
            return v0 + (v1 - v0) * (t - t0) / (t1 - t0)
    return points[-1][1]


def state(t):
    """Every quantity of the car at time t."""
    step = 1.0 / RATE_HZ
    speed = along(SPEED, t)                                             # km/h
    accel = (along(SPEED, t + step) - along(SPEED, t - step)) / 3.6 / (2 * step)   # m/s^2
    angle = along(STEERING, t)                                          # degrees at the wheel
    angle_speed = abs(along(STEERING, t + step) - along(STEERING, t - step)) / (2 * step)
    yaw = speed / 3.6 / WHEELBASE_M * math.tan(math.radians(angle / STEERING_RATIO))   # rad/s
    inner, outer = speed - yaw * TRACK_M / 2 * 3.6, speed + yaw * TRACK_M / 2 * 3.6
    braking = accel < -0.5
    return {
        "CLU11": {"CF_Clu_Vanz": speed},
        "WHL_SPD11": {"WHL_SPD_FL": inner, "WHL_SPD_FR": outer, "WHL_SPD_RL": inner, "WHL_SPD_RR": outer},
        "ESP12": {
            "LONG_ACCEL": accel,
            "LAT_ACCEL": speed / 3.6 * yaw,
            "YAW_RATE": math.degrees(yaw),
            "CYL_PRES": -12.0 * accel if braking else 0.0,
        },
        "SAS11": {"SAS_Angle": angle, "SAS_Speed": angle_speed},
        "EMS12": {
            "BRAKE_ACT": 1 if braking else 0,
            "PV_AV_CAN": 0.0 if braking else 8.0 + 15.0 * max(accel, 0.0),
            "TPS": 0.0 if braking else 6.0 + 12.0 * max(accel, 0.0),
        },
        "EMS11": {"N": 800.0 + 35.0 * speed, "TQI": 0.0 if braking else 20.0 + 20.0 * max(accel, 0.0), "VS": speed},
        "TCU12": {
            "CUR_GR": 2 if speed < 20 else 3 if speed < 40 else 4 if speed < 60 else 5,
            # The transmission sends the speed in two signals: whole km/h and the rest.
            "VS_TCU": math.floor(speed),
            "VS_TCU_DECIMAL": speed - math.floor(speed),
        },
        "MDPS12": {"CR_Mdps_StrColTq": 0.02 * angle},
        "SCC11": {"ACC_ObjDist": 40.0, "ACC_ObjRelSpd": 0.0, "ACC_ObjLatPos": 0.0},
    }


def main():
    dbc_path, out_path = sys.argv[1], sys.argv[2]
    db = cantools.database.load_file(dbc_path, strict=False)
    lines = []
    for i in range(RATE_HZ * SECONDS):
        t = i / RATE_HZ
        for k, (name, values) in enumerate(state(t).items()):
            message = db.get_message_by_name(name)
            # Every other signal of the message stays at its raw zero.
            data = {s.name: s.offset for s in message.signals}
            for signal_name, value in values.items():
                signal = message.get_signal_by_name(signal_name)
                raw = round((value - signal.offset) / signal.scale)
                top = (1 << (signal.length - 1)) - 1 if signal.is_signed else (1 << signal.length) - 1
                bottom = -top - 1 if signal.is_signed else 0
                data[signal_name] = max(bottom, min(top, raw)) * signal.scale + signal.offset
            payload = message.encode(data, strict=False)
            # The nine messages of one instant leave 1 ms apart, like frames sharing one bus.
            lines.append(f"({START + t + 0.001 * k:.6f}) vcan0 {message.frame_id:03X}#{payload.hex().upper()}\n")
    with open(out_path, "w") as out:
        out.writelines(lines)
    print(f"{len(lines)} frames in {SECONDS} s written to {out_path}")


if __name__ == "__main__":
    main()
