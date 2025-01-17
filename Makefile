DOCKER_SERVICES ?= all

help:
	@grep '^[^#[:space:]].*:' Makefile

# Usage:
# `make docker-compose-up` starts all the services.
# `make docker-compose-up DOCKER_SERVICES='jaeger,localstack'` starts the subset of services matching the profiles.
docker-compose-up:
	@echo "Launching ${DOCKER_SERVICES} Docker service(s)"
	COMPOSE_PROFILES=$(DOCKER_SERVICES) docker-compose -f docker-compose.yml up -d --remove-orphans

docker-compose-down:
	docker-compose -f docker-compose.yml down --remove-orphans

docker-compose-logs:
	docker-compose logs -f -t

license-check:
	docker run -it --rm -v $(shell pwd):/github/workspace ghcr.io/apache/skywalking-eyes/license-eye header check

license-fix:
	docker run -it --rm -v $(shell pwd):/github/workspace ghcr.io/apache/skywalking-eyes/license-eye header fix

fmt:
	@echo "Formatting Rust files"
	@(rustup toolchain list | ( ! grep -q nightly && echo "Toolchain 'nightly' is not installed. Please install using 'rustup toolchain install nightly'.") ) || cargo +nightly fmt

# Usage:
# `make test-all` starts the Docker services and runs all the tests.
# `make -k test-all docker-compose-down`, tears down the Docker services after running all the tests.
test-all: docker-compose-up
	QUICKWIT_ENV=local cargo test --all-features

# This will build and push all custom cross images for cross-compilation.
# You will need to login into dockerhub with `quickwitinc` account
IMAGE_TAGS = x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-unknown-linux-musl aarch64-unknown-linux-musl 

.PHONY: cross-images
cross-images:
	for tag in ${IMAGE_TAGS}; do \
		docker build --tag quickwitinc/builder-$$tag --file ./build/cross-images/$$tag.dockerfile ./build/cross-images; \
		docker push quickwitinc/builder-$$tag:latest; \
	done
