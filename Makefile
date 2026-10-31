export HOST_UID := $(shell id -u)
export HOST_GID := $(shell id -g)
export SCALE ?= 2
export GEOMETRY ?= 3840x2160
DC = docker compose

.PHONY: image test lint run dist portable-check test-x11 test-hidpi test-editor test-guide coverage test-all package package-check icons
image: ; $(DC) build test
test:  ; $(DC) run --rm test
lint:  ; $(DC) run --rm lint
run:   ; $(DC) run --rm run
dist:  ; $(DC) run --rm dist

portable-check: ; sh scripts/portable-check.sh
test-x11: ; $(DC) run --rm test-x11
test-hidpi: ; $(DC) run --rm test-hidpi
test-editor: ; $(DC) run --rm test-editor
test-guide: ; $(DC) run --rm test-guide
coverage: ; $(DC) run --rm coverage
# Everything, in the order of the release gate (prerequisites run one after another).
test-all: lint test test-x11 test-guide test-editor test-hidpi dist portable-check package package-check
package: ; $(DC) run --rm package
icons: ; $(DC) run --rm icons
package-check: ; sh scripts/package-check.sh
