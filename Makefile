# jedge-stream: the decoded signals of a CAN recording, from the vehicle Pi to the gateway Pi,
# in real time. Run `make` alone to see the commands.
#
# Every setting below can be changed on the command line, for example:
#     make send LOG=data/dump2.log DURATION=60

GATEWAY  ?= 10.77.0.1
PORT     ?= 7700
LOG      ?= data/dump1.log
DBC      ?= hyundai_2015_ccan.dbc
SPEED    ?= 1
DURATION ?=
WATCH    ?= WHL_SPD11.WHL_SPD_FL,ESP12.LONG_ACCEL,SAS11.SAS_Angle
SAVE     ?=

BIN := target/release

# The DBC of the car, from the opendbc project of comma.ai, pinned to one commit.
DBC_COMMIT := 79ddcc7bb72d557fc5b20d038c0fad9c9d423446
DBC_SHA256 := d7f822e3a8d220cfde83aec2ae4ed28ddbc44a38ff29bea2e12e3e321d73a607
DBC_URL    := https://raw.githubusercontent.com/commaai/opendbc/$(DBC_COMMIT)/opendbc/dbc/hyundai_2015_ccan.dbc

.DEFAULT_GOAL := help
.PHONY: help build test dbc demo receive send expected info clean

help:
	@echo "both       make build      compile the two programs"
	@echo "both       make test       run the unit tests"
	@echo "both       make demo       both ends on this machine, with 10 s of sample traffic"
	@echo "both       make info       what this machine has: commit, programs, DBC, addresses"
	@echo "gateway    make receive    wait for the vehicle and show what arrives"
	@echo "vehicle    make send       decode LOG and send it to the gateway in real time"
	@echo "vehicle    make expected   write expected.ndjson: what the gateway must receive"
	@echo "vehicle    make dbc        download the DBC and check its sha256"
	@echo
	@echo "computer   make setup GATEWAY_IP=... VEHICLE_IP=...   prepare the two Pis, then the menu"
	@echo "computer   make menu       choose what to do with the two Pis"
	@echo "computer   make run        start both ends and show what the gateway receives"
	@echo "computer   make stop       stop both ends"
	@echo "computer   make status     what each Pi has, and whether the vehicle reaches the gateway"
	@echo "computer   make update     bring the latest code to the two Pis and build it"
	@echo "computer   make data FILE=recording.log        copy a recording to the vehicle"
	@echo "computer   make compare SAVE=received.ndjson   check that every byte arrived"
	@echo
	@echo "settings: GATEWAY=$(GATEWAY) PORT=$(PORT) LOG=$(LOG) DBC=$(DBC) SPEED=$(SPEED)"
	@echo "          DURATION=$(DURATION) (seconds; empty sends the whole recording)"
	@echo "          SAVE=$(SAVE) (file where make receive also stores what arrives)"
	@echo "          WATCH=$(WATCH)"
	@echo "bench:    GATEWAY_IP=$(GATEWAY_IP) VEHICLE_IP=$(VEHICLE_IP) PI_USER=$(PI_USER) DIR=$(DIR)"

build:
	cargo build --release

test:
	cargo test

dbc: hyundai_2015_ccan.dbc

# The file only gets its name once the checksum has matched.
hyundai_2015_ccan.dbc:
	curl -sSfL -o $@.part $(DBC_URL)
	echo "$(DBC_SHA256)  $@.part" | sha256sum -c -
	mv $@.part $@

# The receiver goes to the background and the sender keeps trying until it is listening.
demo: build $(DBC)
	@$(BIN)/signal_receiver --listen 127.0.0.1:$(PORT) --watch $(WATCH) & \
	$(BIN)/signal_sender --log sample/sample.log --dbc $(DBC) --to 127.0.0.1:$(PORT) 2> .demo_sender.txt; \
	wait; \
	sed -n '/^--- signal_sender/,$$p' .demo_sender.txt; \
	rm -f .demo_sender.txt

receive: build
	$(BIN)/signal_receiver --listen 0.0.0.0:$(PORT) --watch $(WATCH) $(if $(SAVE),--save $(SAVE))

send: build $(DBC)
	$(BIN)/signal_sender --log $(LOG) --dbc $(DBC) --to $(GATEWAY):$(PORT) --speed $(SPEED) $(if $(DURATION),--duration $(DURATION))

expected: build $(DBC)
	$(BIN)/signal_sender --log $(LOG) --dbc $(DBC) --stdout --speed 0 $(if $(DURATION),--duration $(DURATION)) > expected.ndjson
	sha256sum expected.ndjson

# What this machine has. `make status` asks both Pis for it.
info:
	@echo "$$(hostname): $$(git log -1 --format='commit %h, %s' 2> /dev/null || echo 'not a git clone')"
	@test -x $(BIN)/signal_sender && echo "  programs: built" || echo "  programs: not built"
	@test -f $(DBC) && echo "  DBC: here" || echo "  DBC: not here (only the vehicle needs it)"
	@pgrep -x signal_receiver > /dev/null && echo "  signal_receiver: running" || echo "  signal_receiver: stopped"
	@pgrep -x signal_sender > /dev/null && echo "  signal_sender: running" || echo "  signal_sender: stopped"
	@ip -4 -br addr show 2> /dev/null | grep -v '^lo ' | sed 's/^/  address: /'

clean:
	cargo clean
	rm -f expected.ndjson received.ndjson .demo_sender.txt .sender.txt

# ---------------------------------------------------------------------------------------------
# From your computer: the two Pis, driven through ssh.
#
#     make setup GATEWAY_IP=192.168.1.50 VEHICLE_IP=192.168.1.51
#
# GATEWAY_IP and VEHICLE_IP are the addresses at which your computer reaches each Pi. They are
# stored in bench.mk, so they are typed once. GATEWAY and VEHICLE are something else: the
# addresses on the cable between the two Pis, where the signals travel.

-include bench.mk

PI_USER  ?= pi
DIR      ?= bench/jedge-stream
REPO     ?= $(shell git config --get remote.origin.url)
VEHICLE  ?= 10.77.0.2
IFACE    ?= eth0
SSH_OPTS ?=
FILE     ?=

# One connection to each Pi stays open for ten minutes, so the password is asked once.
SSH        := ssh $(SSH_OPTS) -o ControlMaster=auto -o ControlPath=~/.ssh/jedge-%C -o ControlPersist=10m
GATEWAY_PI := $(PI_USER)@$(GATEWAY_IP)
VEHICLE_PI := $(PI_USER)@$(VEHICLE_IP)
# make on a Pi, inside the project. ssh does not put ~/.cargo/bin, where rustup leaves cargo, in PATH.
PI_MAKE    := cd $(DIR) && PATH=$$HOME/.cargo/bin:$$PATH make --no-print-directory
# The settings of a run, handed to make on each Pi.
RUN        := LOG=$(LOG) SPEED=$(SPEED) DURATION=$(DURATION) GATEWAY=$(GATEWAY) PORT=$(PORT) WATCH=$(WATCH) SAVE=$(SAVE)

.PHONY: setup update menu run stop status data compare need-bench

need-bench:
	@test -n "$(GATEWAY_IP)" && test -n "$(VEHICLE_IP)" || { echo "first: make setup GATEWAY_IP=... VEHICLE_IP=...   (the addresses at which this computer reaches each Pi)"; exit 1; }

# Stores the addresses, prepares the two Pis and shows the menu.
setup: need-bench
	@printf '%s\n' 'GATEWAY_IP := $(GATEWAY_IP)' 'VEHICLE_IP := $(VEHICLE_IP)' 'PI_USER := $(PI_USER)' \
		'DIR := $(DIR)' 'GATEWAY := $(GATEWAY)' 'VEHICLE := $(VEHICLE)' 'IFACE := $(IFACE)' \
		'SSH_OPTS := $(SSH_OPTS)' > bench.mk
	@echo "the addresses of this bench are stored in bench.mk"
	@$(MAKE) --no-print-directory update
	@$(MAKE) --no-print-directory menu

# Sends scripts/setup_pi.sh to each Pi through ssh and runs it there. It can be run again:
# each step only acts if something is missing, and the code is brought up to date.
update: need-bench
	@test -n "$(REPO)" || { echo "REPO is empty: this folder has no git remote. Push it to a git server, or pass REPO=address"; exit 1; }
	@$(SSH) $(GATEWAY_PI) 'bash -s -- gateway $(REPO) $(DIR) $(GATEWAY)/24 $(IFACE)' < scripts/setup_pi.sh
	@$(SSH) $(VEHICLE_PI) 'bash -s -- vehicle $(REPO) $(DIR) $(VEHICLE)/24 $(IFACE)' < scripts/setup_pi.sh
	@echo
	@$(SSH) $(VEHICLE_PI) 'ping -c 3 $(GATEWAY)' || { echo "the vehicle does not reach the gateway at $(GATEWAY): check the cable, or pass GATEWAY=address"; exit 1; }

menu: need-bench
	@while true; do \
		echo; \
		echo "  1) run the 10 s sample from the vehicle to the gateway"; \
		echo "  2) run the recording: LOG=$(LOG) DURATION=$(DURATION)"; \
		echo "  3) status of the two Pis"; \
		echo "  4) stop both ends"; \
		echo "  5) bring the latest code to the two Pis and build it"; \
		echo "  6) list every command"; \
		echo "  q) quit"; \
		printf "choice: "; \
		read choice || exit 0; \
		case "$$choice" in \
			1) $(MAKE) --no-print-directory run LOG=sample/sample.log DURATION= ;; \
			2) $(MAKE) --no-print-directory run ;; \
			3) $(MAKE) --no-print-directory status ;; \
			4) $(MAKE) --no-print-directory stop ;; \
			5) $(MAKE) --no-print-directory update ;; \
			6) $(MAKE) --no-print-directory help ;; \
			q) exit 0 ;; \
			*) echo "not an option: $$choice" ;; \
		esac; \
	done

# The sender starts first, in the background, and waits for the gateway. The receiver stays in
# front, so what arrives is seen live. Both end by themselves when the recording ends. If the
# receiver ends and the sender is still there, it is stopped, so nothing is left waiting.
run: need-bench
	@$(SSH) $(VEHICLE_PI) 'cd $(DIR) && test -f $(LOG)' || { echo "the vehicle does not have $(LOG). Copy it with: make data FILE=...   or name another with LOG=..."; exit 1; }
	@$(MAKE) --no-print-directory stop > /dev/null
	@$(SSH) -n $(VEHICLE_PI) '$(PI_MAKE) send $(RUN)' > .sender.txt 2>&1 & \
	$(SSH) -t $(GATEWAY_PI) '$(PI_MAKE) receive $(RUN)'; \
	sleep 1; \
	$(SSH) $(VEHICLE_PI) 'pkill -x signal_sender' > /dev/null; \
	wait; \
	grep -A 20 '^--- signal_sender' .sender.txt || tail -n 5 .sender.txt; \
	rm -f .sender.txt

stop: need-bench
	@$(SSH) $(VEHICLE_PI) 'pkill -x signal_sender' && echo "vehicle: signal_sender stopped" || echo "vehicle: signal_sender not running"
	@$(SSH) $(GATEWAY_PI) 'pkill -x signal_receiver' && echo "gateway: signal_receiver stopped" || echo "gateway: signal_receiver not running"

status: need-bench
	@$(SSH) $(GATEWAY_PI) '$(PI_MAKE) info'
	@$(SSH) $(VEHICLE_PI) '$(PI_MAKE) info'
	@$(SSH) $(VEHICLE_PI) 'ping -c 1 -W 2 $(GATEWAY) > /dev/null' && echo "cable: the vehicle reaches the gateway at $(GATEWAY)" || echo "cable: the vehicle does NOT reach the gateway at $(GATEWAY)"

# Copies a recording from this computer to the data/ folder of the vehicle.
data: need-bench
	@test -n "$(FILE)" && test -f $(FILE) || { echo "usage: make data FILE=path/to/recording.log"; exit 1; }
	$(SSH) $(VEHICLE_PI) 'mkdir -p $(DIR)/data && cat > $(DIR)/data/$(notdir $(FILE))' < $(FILE)
	@echo "it is on the vehicle. Use it with: make run LOG=data/$(notdir $(FILE))"

# The sha256 of what the gateway stored, against the sha256 of what the vehicle had to send.
compare: need-bench
	@test -n "$(SAVE)" || { echo "usage: make run SAVE=received.ndjson   and then   make compare SAVE=received.ndjson"; exit 1; }
	@expected=$$($(SSH) $(VEHICLE_PI) '$(PI_MAKE) -s expected $(RUN) 2> /dev/null' | cut -d' ' -f1); \
	received=$$($(SSH) $(GATEWAY_PI) 'cd $(DIR) && sha256sum $(SAVE)' | cut -d' ' -f1); \
	echo "expected  $$expected"; \
	echo "received  $$received"; \
	test -n "$$expected" && test "$$expected" = "$$received" && echo "same: every byte arrived" || { echo "DIFFERENT"; exit 1; }
