GOOS   ?= linux
OUT    ?= bin

CMDS = ntp-health-agent ntp-enrollment-prehook

# Multi-arch by default. Ansible roles pick bin/linux-<arch>/<cmd> based on
# target host architecture at deploy time (x86_64 → amd64, aarch64 → arm64).
# Building both keeps the repo deployable to current arm64 Orins AND future
# amd64 Karbon 801s without re-tooling.
ARCHES ?= amd64 arm64

.PHONY: all build build-arch clean test vet

all: build

build:
	@for arch in $(ARCHES); do \
		$(MAKE) --no-print-directory build-arch GOARCH=$$arch; \
	done

build-arch:
	@if [ -z "$(GOARCH)" ]; then echo "ERROR: set GOARCH"; exit 1; fi
	@mkdir -p $(OUT)/$(GOOS)-$(GOARCH)
	@for cmd in $(CMDS); do \
		if [ -d cmd/$$cmd ]; then \
			GOOS=$(GOOS) GOARCH=$(GOARCH) CGO_ENABLED=0 \
				go build -o $(OUT)/$(GOOS)-$(GOARCH)/$$cmd ./cmd/$$cmd/; \
			echo "built $(OUT)/$(GOOS)-$(GOARCH)/$$cmd"; \
		fi; \
	done

test:
	go test ./...

vet:
	go vet ./...

clean:
	rm -rf $(OUT)
