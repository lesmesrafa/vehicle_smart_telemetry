# jedge-stream

Sends every decoded signal of a CAN recording from the vehicle Pi to the gateway Pi, at the pace at
which the car produced it.

```
vehicle Pi (16 GB)                                       gateway Pi (1 GB)
recording.log --> signal_sender ------ Ethernet ------> signal_receiver --> (knowledge graph, later)
                  decodes with the DBC   TCP, port 7700   keeps the latest value of every signal
                  waits for each frame's moment
```

- `signal_sender` reads a recording in candump format, decodes each frame with the DBC and sends
  one line per CAN message. A frame never leaves before its moment in the recording.
- `signal_receiver` listens, receives the lines and keeps the latest value of every signal. It
  reports what arrives and how faithful the timing is.

There are no external libraries: only the Rust standard library. It builds without internet.

## What goes on each Pi

| | Vehicle (16 GB) | Gateway (1 GB) |
|---|---|---|
| Program | `signal_sender` | `signal_receiver` |
| Command | `make send` | `make receive` |
| Needs | the recording as a `.log`, the DBC | nothing else: no DBC, no dataset, no Python |

Both Pis get the whole project and build it. Rust 1.77 or later (`cargo --version`). Section 2
does all of it from your computer; section 3 is the same by hand.

## 1. Try it on one machine

```bash
make build
make test      # test result: ok. 8 passed
make demo      # needs internet once, to download the DBC
```

`make demo` runs both ends on this machine with `sample/sample.log`: 10 s of synthetic traffic, not
a recording of a car. It takes 10 s and both summaries must give the same two figures:

```
messages received: 21860 with 199000 signal values
messages sent: 21860 with 199000 signal values, from 62 identifiers
```

## 2. Two Raspberry Pis, from your computer

The project has to be in a git repository that the Pis can clone, and your computer needs `make`,
`git` and `ssh` (Linux, macOS or WSL). Give it the addresses at which it reaches each Pi:

```bash
make setup GATEWAY_IP=192.168.1.50 VEHICLE_IP=192.168.1.51
```

On each Pi, through ssh, that command:

1. installs git, curl, make and a C compiler if any is missing;
2. installs Rust with rustup if there is none or it is older than 1.77;
3. clones the project into `~/bench/jedge-stream`, or brings what is new if it is already there;
4. gives the Pi its address on the cable between the two (`10.77.0.1` the gateway, `10.77.0.2` the
   vehicle) if it does not have it yet;
5. builds the two programs and, on the vehicle, downloads the DBC.

It ends by checking that the vehicle reaches the gateway, and shows a menu:

```
  1) run the 10 s sample from the vehicle to the gateway
  2) run the recording: LOG=data/dump1.log DURATION=
  3) status of the two Pis
  4) stop both ends
  5) bring the latest code to the two Pis and build it
  6) list every command
  q) quit
```

The addresses are stored in `bench.mk`, which git ignores, so they are typed once. After that:

| Command | What it does |
|---|---|
| `make menu` | the menu again |
| `make run` | starts both ends and shows what the gateway receives; `LOG`, `DURATION`, `SPEED`, `SAVE` and `WATCH` apply |
| `make stop` | stops both ends |
| `make status` | commit, programs, DBC and addresses of each Pi, and whether the vehicle reaches the gateway |
| `make update` | steps 1 to 5 again: what the Pis get is what has been pushed to the git server |
| `make data FILE=dump1.log` | copies a recording from your computer to `data/` on the vehicle |
| `make compare SAVE=received.ndjson` | after `make run SAVE=received.ndjson`: checks that every byte arrived |

Settings of `make setup`, all stored in `bench.mk`:

| Setting | Default | Meaning |
|---|---|---|
| `GATEWAY_IP`, `VEHICLE_IP` | none | where your computer reaches each Pi: an address or a name such as `gateway.local` |
| `PI_USER` | `pi` | the user on both Pis |
| `DIR` | `bench/jedge-stream` | folder of the project on each Pi, inside the home folder |
| `GATEWAY`, `VEHICLE` | `10.77.0.1`, `10.77.0.2` | addresses on the cable between the two Pis, where the signals travel |
| `IFACE` | `eth0` | network port of that cable |
| `REPO` | the `origin` of this clone | git address the Pis clone from |
| `SSH_OPTS` | empty | extra options for ssh, for example `-A` to clone a private repository with your own key |

- ssh asks for the password of each Pi once: the connection stays open for ten minutes.
- sudo must not ask for a password on the Pis, which is how Raspberry Pi OS comes.
- Your computer has to stay on and connected during a run: both programs hang from its ssh
  sessions. `Ctrl+C` stops the run on both Pis.
- The port of the cable is never reconfigured when the ssh session itself comes in through it.

## 3. Two Raspberry Pis, by hand

The vehicle needs the recording in candump format, one frame per line: `(time) bus ID#DATA`. For
X-CANIDS that is the `data/dump1.log` that `parquet_to_candump.py` writes from `raw/dump1.parquet`.

**[gateway]**

```bash
make receive
```

**[vehicle]**

```bash
make dbc
make send LOG=data/dump1.log
```

- The order does not matter: the sender waits until the gateway is listening.
- The gateway is `10.77.0.1`. If yours has another address: `make send GATEWAY=10.77.0.2`.
- `make send DURATION=60` sends only the first minute.
- The run lasts as long as the recording. It ends by itself and each end prints a summary.
- To stop early, press `Ctrl+C` on the vehicle. The gateway then prints its summary.

The gateway prints one line per second:

```
t=    9.0 s |  2177 messages/s |  19817 values/s |  579 signals | timing error up to  0.99 ms | WHL_SPD_FL=98.03 LONG_ACCEL=-0.72 SAS_Angle=-90.00
```

- `t`: time inside the recording.
- `signals`: how many different signals have arrived so far.
- `timing error`: how much later a message arrived than the recording says, in the worst case of
  that second. Zero is the message that has travelled fastest so far, so the two clocks need not
  be synchronised.
- The last columns are the signals listed in `WATCH`.

## 4. Check that everything arrived

Compare the two summaries: `messages` and `signal values` must be equal on both ends.

To compare the content byte for byte:

```bash
# [gateway]
make receive SAVE=received.ndjson
# [vehicle]
make send
make expected              # writes expected.ndjson without waiting and prints its sha256
# [gateway], when the run has ended
sha256sum received.ndjson
```

From your computer the same check is `make run SAVE=received.ndjson` and then
`make compare SAVE=received.ndjson`, which prints both values and says whether they are the same.

The two sha256 values must be the same. The size of `received.ndjson` is also the number of bytes
that crossed the cable. With the test traffic each file grows 0.5 MB per second: a 30-minute
recording is about 0.9 GB.

## The wire format

One line per CAN message, in JSON:

```json
{"t":12.345678,"id":544,"msg":"ESP12","sig":{"LAT_ACCEL":0.03,"LONG_ACCEL":-0.13}}
```

| Field | Meaning |
|---|---|
| `t` | seconds since the first frame of the recording |
| `id` | CAN identifier, in decimal |
| `msg` | name of the message in the DBC |
| `sig` | every signal that frame carries, as a physical value (factor and offset applied) |

- Units, senders and receivers are not sent: they do not change, and they are in the DBC.
- A multiplexed signal appears only in the frames that carry it.
- A frame whose identifier is not in the DBC cannot be decoded and is not sent. The sender counts
  those frames and lists their identifiers in its summary.
- Values travel as 64-bit floating point numbers. A signal wider than 53 bits loses its last
  digits. In the Hyundai DBC there is one, `ACU12.CR_Acu_SN`, a serial number.

## Where the knowledge graph goes

In `src/bin/signal_receiver.rs`, the function `State::accept` receives every message as a `Sample`
(time, identifier, message name and the list of signals with their values). Today it stores the
latest value of each signal. That is the place from which the graph will be fed.

A program in another language can also read the stream directly: it is plain text over TCP, and
any JSON library reads the lines.

## Options

Each program lists its options with `--help`.

| `signal_sender` | |
|---|---|
| `--log FILE` | the recording (required) |
| `--dbc FILE` | DBC file (default `hyundai_2015_ccan.dbc`) |
| `--to HOST:PORT` | where the gateway listens (default `10.77.0.1:7700`) |
| `--speed X` | 1 is real time, 2 is twice as fast, 0 does not wait |
| `--duration S` | send only the first S seconds |
| `--stdout` | write the lines to standard output instead of sending them |

| `signal_receiver` | |
|---|---|
| `--listen ADDR:PORT` | where to listen (default `0.0.0.0:7700`) |
| `--save FILE` | also write every line received to FILE, unchanged |
| `--watch LIST` | signals to show, as `MESSAGE.SIGNAL` separated by commas |
| `--every S` | seconds between reports (1) |
| `--quiet` | only the final summary |

## Files

| File | What it is |
|---|---|
| `src/bin/signal_sender.rs` | the program of the vehicle |
| `src/bin/signal_receiver.rs` | the program of the gateway |
| `src/wire.rs` | the wire format: `encode`, `decode` and `Sample` |
| `src/timing.rs` | statistics of the delays |
| `src/candump.rs`, `src/dbc.rs`, `src/options.rs`, `src/metrics.rs` | taken from `jedge-bench` |
| `sample/sample.log` | 10 s of synthetic traffic for `make demo` |
| `scripts/setup_pi.sh` | prepares one Pi; `make setup` and `make update` send it through ssh |
| `bench.mk` | the addresses of your bench, written by `make setup`; git ignores it |

## What has been run and what has not

Run on 2026-11-24 on Linux x86-64 with Rust 1.97, both ends on the same machine:

- 120 s of synthetic traffic in real time: 62 identifiers, 2186 messages per second. The gateway
  received all 262,320 messages and 2,388,000 values, and the bytes saved were the same as the
  expected ones.
- The same values decoded with cantools 45.2.0: 2,382,020 equal to the last digit. The other 5,980
  belong to `ACU12.CR_Acu_SN`, the 64-bit signal described above.
- Size in that run: 220.6 bytes per message, 482 kB per second.
- Timing in that run: frames left 0.12 ms late on average and arrived with a mean timing error of
  0.19 ms. About 1 % of the messages were more than 1 ms late and the worst was 17 ms. The machine
  was a shared virtual one with two cores.
- Cost of the receiver in that run: 4.4 s of CPU in 120 s and 2.4 MB of memory at most. The memory
  does not grow with the length of the recording: the receiver keeps one value per signal.
- Stopping either end in the middle, `--duration`, `--speed`, and `make demo` from a clean copy.

The commands of section 2 were run the same day against two ssh servers on one Linux machine, each
with its own home folder and host name and the same user, standing in for the two Pis:

- `make setup` with a password and with a key (ssh asked for each password once), `make update`
  after a new commit, `make status`, `make run` with the sample (21860 messages and 199000 values
  on both ends), `Ctrl+C` in the middle of a run, a run whose sender cannot reach the gateway,
  `make stop`, `make data`, and `make compare` with equal and with different recordings.
- In `scripts/setup_pi.sh`: the clone and the later pull, the installation with apt-get when a tool
  is missing, the stop when sudo asks for a password, the stop when the folder exists and is not a
  clone, and the detection of an old Rust.

Not run:

- **Anything on a Raspberry Pi**, or between two machines over a cable.
- **The installation of Rust with rustup**: the test machine cannot reach it. The command is the
  one in the rustup documentation.
- **NetworkManager.** The `nmcli` commands that set the address of the cable were checked only
  against a stand-in that records how it is called.
- **Cloning a private repository** with `SSH_OPTS=-A`. What was checked is that the option makes
  your key available on the Pi.
- **macOS and WSL** as the computer that runs `make setup`.
- **X-CANIDS.** The traffic used was generated from the DBC, with random bytes in most messages.
- **Rust older than 1.97.** By what the code uses, 1.77 should be enough.
