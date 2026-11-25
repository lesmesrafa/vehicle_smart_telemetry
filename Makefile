# vehicle_smart_telemetry: the decoded signals of a CAN recording, from the vehicle Pi to the gateway Pi,
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
# The knowledge graph of the gateway. With GRAPH empty the receiver works without it.
GRAPH     ?= signal_graph.nt
QUESTIONS ?= questions.tsv
# The quantities of the signals graph_builder does not name. With DEFAULTS empty the graph has
# only the quantities written in graph_builder.
DEFAULTS  ?= default_quantities.tsv
PROMPTS   ?=
RDF       ?=
JELLY     ?=
# The language model of the gateway: llama-server of llama.cpp, with one slot per question.
# `make llama` builds the program and `make weights` downloads the model, both into this folder.
LLAMA_DIR   ?= llama.cpp
LLAMA       ?= $(LLAMA_BUILT)
MODEL       ?= $(MODEL_FILE)
JOBS        ?= 4
# The memory, in MB, below which this machine does not try to build llama.cpp.
BUILD_MIN_MB ?= 3000
MODEL_PORT  ?= 8080
# The seconds to wait for the model to load when it is started behind, by make model-up.
MODEL_WAIT  ?= 120
SLOTS       ?= 6
SLOT_TOKENS ?= 1024
THREADS     ?= 4
ANSWERS     ?= answers.ndjson
# With FOLLOW=1, make ask keeps reading PROMPTS while the receiver writes it, and stops after
# IDLE seconds with nothing new.
FOLLOW      ?=
IDLE        ?= 60

BIN := target/release

# llama.cpp, the program that runs the model: its source, pinned to one commit, and where the
# server is once built.
LLAMA_REPO   := https://github.com/ggml-org/llama.cpp
LLAMA_COMMIT := de306972853640223d20ce57a3d9e44f6719d3db
LLAMA_BUILT  := $(LLAMA_DIR)/build/bin/llama-server

# The weights of LFM2.5-230M at 4 bits, from the repository of Liquid AI on Hugging Face, pinned
# to one commit. The sha256 is the one that repository gives for the file, 149080928 bytes.
MODEL_FILE   := models/LFM2.5-230M-Q4_0.gguf
MODEL_COMMIT := fa224d4cb60cffe61eb58726712ef255bb64d0b7
MODEL_SHA256 := 430fbec5b1b355e9bb12cd0638c9f2a8f21fedd6eafb4103e42c7e88887daa73
MODEL_URL    := https://huggingface.co/LiquidAI/LFM2.5-230M-GGUF/resolve/$(MODEL_COMMIT)/LFM2.5-230M-Q4_0.gguf

# The DBC of the car, from the opendbc project of comma.ai, pinned to one commit.
DBC_COMMIT := 79ddcc7bb72d557fc5b20d038c0fad9c9d423446
DBC_SHA256 := d7f822e3a8d220cfde83aec2ae4ed28ddbc44a38ff29bea2e12e3e321d73a607
DBC_URL    := https://raw.githubusercontent.com/commaai/opendbc/$(DBC_COMMIT)/opendbc/dbc/hyundai_2015_ccan.dbc

.DEFAULT_GOAL := help
.PHONY: help build test dbc graph demo demo-graph receive send expected llama weights model model-up live ask info clean

# What the receiver is told about the knowledge graph.
KNOWLEDGE = $(if $(GRAPH),--graph $(GRAPH) --questions $(QUESTIONS) $(if $(PROMPTS),--prompts $(PROMPTS)) $(if $(RDF),--rdf $(RDF)) $(if $(JELLY),--jelly $(JELLY)))

# What the model needs before it starts, if it is the program and the weights this Makefile knows
# how to get; with another LLAMA or MODEL, they are yours to bring.
MODEL_NEEDS = $(filter $(LLAMA_BUILT),$(LLAMA)) $(filter $(MODEL_FILE),$(MODEL))
# How the model is started. Each question gets a slot, where the server keeps the state of the
# model after the prefix; the context is shared out evenly among the slots. The cache of old
# prompts in memory is off: on the gateway the memory is for the model.
MODEL_SERVER = $(LLAMA) -m $(MODEL) --host 127.0.0.1 --port $(MODEL_PORT) --parallel $(SLOTS) \
	--ctx-size $$(( $(SLOTS) * $(SLOT_TOKENS) )) --threads $(THREADS) --cache-ram 0 --no-webui
MODEL_HEALTH = curl -sf -o /dev/null http://127.0.0.1:$(MODEL_PORT)/health
# Where the receiver writes what the model reads when make live runs both.
LIVE_PROMPTS = $(if $(PROMPTS),$(PROMPTS),prompts.ndjson)

help:
	@echo "both       make build      compile the two programs"
	@echo "both       make test       run the unit tests"
	@echo "both       make demo       both ends on this machine, with 10 s of sample traffic"
	@echo "both       make demo-graph both ends on this machine, with the sample drive and the knowledge graph"
	@echo "both       make graph      write the knowledge graph of the car, from the DBC"
	@echo "both       make info       what this machine has: commit, programs, DBC, addresses"
	@echo "gateway    make receive    wait for the vehicle and show what arrives"
	@echo "vehicle    make llama      download llama.cpp at its pinned commit and build llama-server"
	@echo "gateway    make weights    download the weights of the model and check their sha256"
	@echo "gateway    make model      start the language model (llama-server) and leave it listening"
	@echo "gateway    make ask        ask the model every window of PROMPTS and show what each answer cost"
	@echo "gateway    make live       model, receiver and questions at once: answers each window as the vehicle sends"
	@echo "vehicle    make send       decode LOG and send it to the gateway in real time"
	@echo "vehicle    make expected   write expected.ndjson: what the gateway must receive"
	@echo "vehicle    make dbc        download the DBC and check its sha256"
	@echo
	@echo "computer   make setup GATEWAY_IP=... VEHICLE_IP=...   prepare the two Pis, then the menu"
	@echo "computer   make menu       choose what to do with the two Pis"
	@echo "computer   make run        start both ends and show what the gateway receives"
	@echo "computer   make run-model  the same with the language model: each answer live, and what the run cost"
	@echo "computer   make stop       stop both ends, and the language model"
	@echo "computer   make status     what each Pi has, and whether the vehicle reaches the gateway"
	@echo "computer   make update     bring the latest code to the two Pis and build it"
	@echo "computer   make model-setup  build llama-server on the vehicle, copy it to the gateway, download the weights there"
	@echo "computer   make data FILE=recording.log        copy a recording to the vehicle"
	@echo "computer   make compare SAVE=received.ndjson   check that every byte arrived"
	@echo
	@echo "settings: GATEWAY=$(GATEWAY) PORT=$(PORT) LOG=$(LOG) DBC=$(DBC) SPEED=$(SPEED)"
	@echo "          DURATION=$(DURATION) (seconds; empty sends the whole recording)"
	@echo "          SAVE=$(SAVE) (file where make receive also stores what arrives)"
	@echo "          WATCH=$(WATCH)"
	@echo "graph:    GRAPH=$(GRAPH) (empty: no knowledge graph) QUESTIONS=$(QUESTIONS)"
	@echo "          DEFAULTS=$(DEFAULTS) (empty: no default quantities)"
	@echo "          files where make receive also writes each window: PROMPTS=$(PROMPTS) RDF=$(RDF) JELLY=$(JELLY)"
	@echo "model:    LLAMA=$(LLAMA) MODEL=$(MODEL) MODEL_PORT=$(MODEL_PORT) THREADS=$(THREADS)"
	@echo "          LLAMA_DIR=$(LLAMA_DIR) JOBS=$(JOBS) (compilers at once in make llama) BUILD_MIN_MB=$(BUILD_MIN_MB)"
	@echo "          SLOTS=$(SLOTS) (one per question) SLOT_TOKENS=$(SLOT_TOKENS) (room for one prefix and one window)"
	@echo "          ANSWERS=$(ANSWERS) FOLLOW=$(FOLLOW) (1: keep reading while the receiver writes) IDLE=$(IDLE)"
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

# The fixed part of the knowledge graph. It is written again every time, so it always matches the
# DBC and the table of default quantities.
graph: build $(DBC)
	$(BIN)/graph_builder --dbc $(DBC) $(if $(DEFAULTS),--defaults $(DEFAULTS)) --out $(GRAPH)

# The receiver goes to the background and the sender keeps trying until it is listening.
demo: build $(DBC)
	@$(BIN)/signal_receiver --listen 127.0.0.1:$(PORT) --watch $(WATCH) & \
	$(BIN)/signal_sender --log sample/sample.log --dbc $(DBC) --to 127.0.0.1:$(PORT) 2> .demo_sender.txt; \
	wait; \
	sed -n '/^--- signal_sender/,$$p' .demo_sender.txt; \
	rm -f .demo_sender.txt

# The same with sample/drive.log, 21 s of a drive whose signals agree with each other, and the
# knowledge graph: the gateway shows what a language model would read for each question.
demo-graph: build graph
	@$(BIN)/signal_receiver --listen 127.0.0.1:$(PORT) --watch CLU11.CF_Clu_Vanz,ESP12.LONG_ACCEL,ESP12.CYL_PRES $(KNOWLEDGE) & \
	$(BIN)/signal_sender --log sample/drive.log --dbc $(DBC) --to 127.0.0.1:$(PORT) 2> .demo_sender.txt; \
	wait; \
	sed -n '/^--- signal_sender/,$$p' .demo_sender.txt; \
	rm -f .demo_sender.txt

receive: build $(if $(GRAPH),graph)
	$(BIN)/signal_receiver --listen 0.0.0.0:$(PORT) --watch $(WATCH) $(if $(SAVE),--save $(SAVE)) $(KNOWLEDGE)

send: build $(DBC)
	$(BIN)/signal_sender --log $(LOG) --dbc $(DBC) --to $(GATEWAY):$(PORT) --speed $(SPEED) $(if $(DURATION),--duration $(DURATION))

expected: build $(DBC)
	$(BIN)/signal_sender --log $(LOG) --dbc $(DBC) --stdout --speed 0 $(if $(DURATION),--duration $(DURATION)) > expected.ndjson
	sha256sum expected.ndjson

llama: $(LLAMA_BUILT)

weights: $(MODEL_FILE)

# Only the commit that is used is downloaded, without the history, and only the server is built,
# as one program that needs no libraries beside it. A machine with little memory does not try:
# the compilers would not fit. What is missing to build is installed, if sudo asks for no password.
$(LLAMA_BUILT):
	@mb=$$(awk '/MemTotal/ { print int($$2 / 1024) }' /proc/meminfo 2> /dev/null); \
	test "$${mb:-0}" -ge $(BUILD_MIN_MB) || { \
		echo "this machine has $${mb:-0} MB of memory, and building llama.cpp is not tried below $(BUILD_MIN_MB) MB."; \
		echo "Build it on the vehicle with make llama and copy $@ here, or from your computer: make model-setup"; \
		exit 1; }
	@missing=; for tool in git cmake c++; do command -v $$tool > /dev/null || missing="$$missing $$tool"; done; \
	test -z "$$missing" || { echo "installing, because these are missing:$$missing"; \
		sudo -n apt-get update -q && sudo -n apt-get install -y -q git cmake g++; }
	@test -d $(LLAMA_DIR)/.git || git init -q $(LLAMA_DIR)
	cd $(LLAMA_DIR) && git fetch -q --depth 1 $(LLAMA_REPO) $(LLAMA_COMMIT) && git checkout -q FETCH_HEAD
	@test "$$(git -C $(LLAMA_DIR) rev-parse HEAD)" = "$(LLAMA_COMMIT)"
	cmake -S $(LLAMA_DIR) -B $(LLAMA_DIR)/build -DBUILD_SHARED_LIBS=OFF -DLLAMA_OPENSSL=OFF > $(LLAMA_DIR)/configure.log
	cmake --build $(LLAMA_DIR)/build -j $(JOBS) --target llama-server
	@$@ --version

# The file only gets its name once the checksum has matched. A download that was cut goes on
# from where it stopped; one that ends with other bytes is thrown away.
$(MODEL_FILE):
	@mkdir -p $(dir $@)
	curl -fL --retry 3 -C - --progress-bar -o $@.part $(MODEL_URL)
	@echo "$(MODEL_SHA256)  $@.part" | sha256sum -c - || { rm -f $@.part; exit 1; }
	@mv $@.part $@

# The language model, in its own process and in front: it shows what it does until Ctrl+C.
model: $(MODEL_NEEDS)
	$(MODEL_SERVER)

# The same behind: it starts unless it already answers, and this waits until it has loaded.
# What the model says goes to model.log.
model-up: $(MODEL_NEEDS)
	@$(MODEL_HEALTH) && { echo "the model was already running"; exit 0; }; \
	echo "starting the model: $(MODEL)"; start=$$(date +%s); \
	($(MODEL_SERVER) > model.log 2>&1 < /dev/null &); \
	for i in $$(seq 1 $(MODEL_WAIT)); do \
		$(MODEL_HEALTH) && { echo "the model is ready after $$(( $$(date +%s) - start )) s"; exit 0; }; \
		sleep 1; pgrep -x llama-server > /dev/null || break; \
	done; \
	echo "the model did not get ready. The last it said, in model.log:"; tail -n 5 model.log; pkill -x llama-server; exit 1

# The whole gateway for one run: the model behind, the receiver behind, writing what the model
# reads, and in front the client, which answers each window as it closes. It ends when the
# receiver ends, which is when the vehicle closes the connection. Then it shows what the run
# cost and stops the model. Stopped with Ctrl+C, it also stops the three.
live: build $(if $(GRAPH),graph) model-up
	@test -n "$(GRAPH)" || { echo "make live needs the knowledge graph, and GRAPH is empty"; exit 1; }
	@trap 'pkill -x signal_receiver; pkill -x model_client; pkill -x llama-server' INT TERM HUP; \
	pkill -x signal_receiver; pkill -x model_client; rm -f $(LIVE_PROMPTS); \
	($(BIN)/signal_receiver --listen 0.0.0.0:$(PORT) --quiet $(if $(SAVE),--save $(SAVE)) --graph $(GRAPH) --questions $(QUESTIONS) \
		--prompts $(LIVE_PROMPTS) $(if $(RDF),--rdf $(RDF)) $(if $(JELLY),--jelly $(JELLY)) > .receiver.txt 2>&1 < /dev/null &); \
	for i in $$(seq 1 50); do test -s $(LIVE_PROMPTS) && break; sleep 0.2; done; \
	echo "waiting for the vehicle on port $(PORT): each window is answered as it closes"; \
	$(BIN)/model_client --prompts $(LIVE_PROMPTS) --answers $(ANSWERS) --server 127.0.0.1:$(MODEL_PORT) --follow --writer signal_receiver; \
	status=$$?; \
	awk '/MemTotal/ { total = $$2 } /MemAvailable/ { free = $$2 } END { printf "gateway: %d MB of its %d MB of memory in use, with the model loaded\n", (total - free) / 1024, total / 1024 }' /proc/meminfo; \
	pkill -x signal_receiver; pkill -x llama-server; \
	grep -q '^--- signal_receiver' .receiver.txt && sed -n '/^--- signal_receiver/,$$p' .receiver.txt || tail -n 5 .receiver.txt; \
	rm -f .receiver.txt; exit $$status

# Without PROMPTS it asks the windows of the sample drive, which come with the project.
ask: build
	$(BIN)/model_client --prompts $(if $(PROMPTS),$(PROMPTS),sample/prompts.ndjson) --answers $(ANSWERS) \
		--server 127.0.0.1:$(MODEL_PORT) $(if $(FOLLOW),--follow --idle $(IDLE))

# What this machine has. `make status` asks both Pis for it.
info:
	@echo "$$(hostname): $$(git log -1 --format='commit %h, %s' 2> /dev/null || echo 'not a git clone')"
	@test -x $(BIN)/signal_sender && echo "  programs: built" || echo "  programs: not built"
	@test -f $(DBC) && echo "  DBC: here" || echo "  DBC: not here (make dbc downloads it)"
	@test -f "$(GRAPH)" && echo "  knowledge graph: $$(wc -l < $(GRAPH)) triples in $(GRAPH)" || echo "  knowledge graph: not written yet (make graph)"
	@pgrep -x signal_receiver > /dev/null && echo "  signal_receiver: running" || echo "  signal_receiver: stopped"
	@command -v $(LLAMA) > /dev/null && echo "  llama-server: here" || echo "  llama-server: not here (make llama builds it; the gateway gets it with make model-setup, from your computer)"
	@test -f $(MODEL) && echo "  model weights: here" || echo "  model weights: not here (make weights downloads them)"
	@pgrep -x llama-server > /dev/null && echo "  llama-server: running" || echo "  llama-server: stopped"
	@pgrep -x signal_sender > /dev/null && echo "  signal_sender: running" || echo "  signal_sender: stopped"
	@ip -4 -br addr show 2> /dev/null | grep -v '^lo ' | sed 's/^/  address: /'

clean:
	cargo clean
	rm -f expected.ndjson received.ndjson .demo_sender.txt .sender.txt .receiver.txt signal_graph.nt answers.ndjson prompts.ndjson model.log

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
DIR      ?= bench/vehicle_smart_telemetry
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
RUN        := LOG=$(LOG) SPEED=$(SPEED) DURATION=$(DURATION) GATEWAY=$(GATEWAY) PORT=$(PORT) WATCH=$(WATCH) SAVE=$(SAVE) \
              GRAPH=$(GRAPH) QUESTIONS=$(QUESTIONS) DEFAULTS=$(DEFAULTS) PROMPTS=$(PROMPTS) RDF=$(RDF) JELLY=$(JELLY)
# The settings of the model, handed to make on the gateway.
MODEL_RUN  := SLOTS=$(SLOTS) SLOT_TOKENS=$(SLOT_TOKENS) THREADS=$(THREADS) MODEL_PORT=$(MODEL_PORT) MODEL_WAIT=$(MODEL_WAIT) ANSWERS=$(ANSWERS)

.PHONY: setup update model-setup menu run run-model stop status data compare need-bench

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

# Everything the model needs, in one go. The vehicle builds llama-server, because the gateway has
# no memory for the compilers; the program goes to the gateway through this computer, and both
# copies must have the same sha256; the gateway runs it once, which shows whether it starts
# there (the one library it may miss is installed), and downloads the weights.
model-setup: need-bench
	@$(SSH) $(VEHICLE_PI) '$(PI_MAKE) llama JOBS=$(JOBS)'
	@$(SSH) $(GATEWAY_PI) 'mkdir -p $(DIR)/$(dir $(LLAMA_BUILT))'
	@$(SSH) $(VEHICLE_PI) 'cat $(DIR)/$(LLAMA_BUILT)' | $(SSH) $(GATEWAY_PI) 'cat > $(DIR)/$(LLAMA_BUILT).part'
	@built=$$($(SSH) $(VEHICLE_PI) 'sha256sum < $(DIR)/$(LLAMA_BUILT)'); \
	copied=$$($(SSH) $(GATEWAY_PI) 'sha256sum < $(DIR)/$(LLAMA_BUILT).part'); \
	test -n "$$built" && test "$$built" = "$$copied" || { echo "llama-server: what reached the gateway is DIFFERENT from what the vehicle built"; exit 1; }
	@echo "llama-server: built on the vehicle and copied to the gateway, same bytes. On the gateway it says:"
	@$(SSH) $(GATEWAY_PI) 'cd $(DIR) && chmod +x $(LLAMA_BUILT).part && mv $(LLAMA_BUILT).part $(LLAMA_BUILT) && \
		{ $(LLAMA_BUILT) --version || { sudo -n apt-get install -y -q libgomp1 && $(LLAMA_BUILT) --version; }; }'
	@$(SSH) $(GATEWAY_PI) '$(PI_MAKE) weights'
	@echo "the model is ready on the gateway: there, make model starts it"

menu: need-bench
	@while true; do \
		echo; \
		echo "  1) run the 10 s sample from the vehicle to the gateway"; \
		echo "  2) run the recording: LOG=$(LOG) DURATION=$(DURATION)"; \
		echo "  3) status of the two Pis"; \
		echo "  4) stop both ends, and the language model"; \
		echo "  5) bring the latest code to the two Pis and build it"; \
		echo "  6) list every command"; \
		echo "  7) run the sample drive: the knowledge graph and what a model would read"; \
		echo "  8) get the language model: llama-server and the weights, onto the gateway"; \
		echo "  9) run the sample drive with the language model: its answers and what they cost"; \
		echo " 10) run the recording with the language model: LOG=$(LOG) DURATION=$(DURATION)"; \
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
			7) $(MAKE) --no-print-directory run LOG=sample/drive.log DURATION= ;; \
			8) $(MAKE) --no-print-directory model-setup ;; \
			9) $(MAKE) --no-print-directory run-model LOG=sample/drive.log DURATION= ;; \
			10) $(MAKE) --no-print-directory run-model ;; \
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

# The same run with the language model answering on the gateway. The model loads first, so the
# vehicle only starts to send when it can be asked. What is seen live is each answer and what it
# cost; at the end, what the model, the receiver and the sender spent, and the cost of each
# answer is copied to this computer.
run-model: need-bench
	@$(SSH) $(VEHICLE_PI) 'cd $(DIR) && test -f $(LOG)' || { echo "the vehicle does not have $(LOG). Copy it with: make data FILE=...   or name another with LOG=..."; exit 1; }
	@$(MAKE) --no-print-directory stop > /dev/null
	@$(SSH) $(GATEWAY_PI) '$(PI_MAKE) model-up $(MODEL_RUN)'
	@$(SSH) -n $(VEHICLE_PI) '$(PI_MAKE) send $(RUN)' > .sender.txt 2>&1 & \
	$(SSH) -t $(GATEWAY_PI) '$(PI_MAKE) live $(RUN) $(MODEL_RUN)'; \
	sleep 1; \
	$(SSH) $(VEHICLE_PI) 'pkill -x signal_sender' > /dev/null; \
	wait; \
	grep -A 20 '^--- signal_sender' .sender.txt || tail -n 5 .sender.txt; \
	rm -f .sender.txt
	@$(SSH) $(GATEWAY_PI) 'cat $(DIR)/$(ANSWERS)' > $(ANSWERS) && echo "what each answer cost is in $(ANSWERS), copied here from the gateway: $$(grep -c '' $(ANSWERS)) answers"

stop: need-bench
	@$(SSH) $(VEHICLE_PI) 'pkill -x signal_sender' && echo "vehicle: signal_sender stopped" || echo "vehicle: signal_sender not running"
	@$(SSH) $(GATEWAY_PI) 'pkill -x signal_receiver' && echo "gateway: signal_receiver stopped" || echo "gateway: signal_receiver not running"
	@$(SSH) $(GATEWAY_PI) 'pkill -x model_client; pkill -x llama-server' && echo "gateway: the language model stopped" || echo "gateway: the language model not running"

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
