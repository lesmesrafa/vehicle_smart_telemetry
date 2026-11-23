# Jedge testbed: one entry point for both Raspberries. The role comes from the host name.
#
#   make          everything this machine needs (the gateway gets no Python)
#   make info     show the role, the other Raspberry and the remote folder
#   make dbc      download the DBC and verify its checksum
#   make check    verify what is installed
#   make bus      start the virtual CAN bus and the tunnel to the other Raspberry
#   make sync     copy the generated files to the other Raspberry
#   make lock     let uv pick the newest versions again and rewrite requirements.txt
#   make clean    remove what make created

# Local overrides, if this bench is not the standard one. Not committed.
-include bench.mk

# "car" or "gateway": the host name given when the card was flashed.
ROLE       ?= $(shell hostname)
CAR_IP     ?= 10.77.0.2
GATEWAY_IP ?= 10.77.0.1
PEER_IP    := $(if $(filter gateway,$(ROLE)),$(CAR_IP),$(GATEWAY_IP))
# Same folder on the other Raspberry, written relative to the home folder.
REMOTE_DIR ?= $(patsubst $(HOME)/%,%,$(CURDIR))

COMMIT   := 79ddcc7bb72d557fc5b20d038c0fad9c9d423446
SHA256   := d7f822e3a8d220cfde83aec2ae4ed28ddbc44a38ff29bea2e12e3e321d73a607
DBC_FILE := hyundai_2015_ccan.dbc
DBC_URL  := https://raw.githubusercontent.com/commaai/opendbc/$(COMMIT)/opendbc/dbc/$(DBC_FILE)

VENV       := .venv
UV         ?= uv
# The Python libraries the vehicle tools ask for. uv turns this line into requirements.txt,
# with every library they pull in pinned to an exact version.
PY_DEPS    := cantools==45.2.0
SYNC_FILES := $(wildcard core_graph.json core_graph.ttl model_signals.tsv synthetic_traffic.log)

.PHONY: all info dbc venv lock build check bus sync clean role

ifeq ($(ROLE),gateway)
all: dbc build
else
all: dbc venv build
endif

info:
	@echo "role: $(ROLE) | other Raspberry: $(PEER_IP) | remote folder: $(REMOTE_DIR)"

role:
	@case "$(ROLE)" in car|gateway) ;; *) echo "error: the host name is '$(ROLE)', not car or gateway; run make ROLE=car or ROLE=gateway" >&2; exit 1;; esac

dbc: $(DBC_FILE)

$(DBC_FILE): Makefile
	curl -fsSL "$(DBC_URL)" -o "$@.tmp"
	@echo "$(SHA256)  $@.tmp" | sha256sum -c --status - \
	  || { rm -f "$@.tmp"; echo "error: the checksum does not match; $@ was not saved" >&2; exit 1; }
	@mv "$@.tmp" "$@"
	@echo "$@ saved ($$(wc -c < $@) bytes); the sha256 checksum matches"

define compile_requirements
@command -v $(UV) >/dev/null || { echo "error: uv not found; install it with: curl -LsSf https://astral.sh/uv/install.sh | sh" >&2; exit 1; }
printf '%s\n' $(PY_DEPS) | $(UV) pip compile --quiet $(1) - -o requirements.txt
@touch requirements.txt
endef

requirements.txt: Makefile
	$(call compile_requirements)

lock:
	$(call compile_requirements,--upgrade)

venv: $(VENV)/.ready

$(VENV)/.ready: requirements.txt
	$(UV) venv --quiet --allow-existing $(VENV)
	$(UV) pip sync --quiet --python $(VENV)/bin/python requirements.txt
	@touch $@

build:
	cd rust && cargo build --release

check: $(DBC_FILE)
	@echo "$(SHA256)  $(DBC_FILE)" | sha256sum -c -
	@cargo --version
	@if [ -x $(VENV)/bin/python ]; then $(VENV)/bin/python -c "import can, cantools; print('python-can', can.__version__, '| cantools', cantools.__version__)"; fi

bus: role
	./bus.sh start $(PEER_IP)

sync: role
	@[ -n "$(SYNC_FILES)" ] || { echo "nothing to copy yet"; exit 1; }
	ssh $(PEER_IP) "mkdir -p $(REMOTE_DIR)"
	scp $(SYNC_FILES) $(PEER_IP):$(REMOTE_DIR)/

clean:
	rm -rf $(VENV) $(DBC_FILE) $(DBC_FILE).tmp