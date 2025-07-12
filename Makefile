# MCP Memory Server Development Makefile

.PHONY: help build test run docker-build docker-run clean lint fmt audit bench

# Default target
help: ## Show this help message
	@echo "MCP Memory Server - Development Commands"
	@echo "========================================"
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*?## "}; {printf "\033[36m%-20s\033[0m %s\n", $$1, $$2}'

# Development
build: ## Build the project
	cargo build

build-release: ## Build optimized release version
	cargo build --release

test: ## Run all tests
	cargo test

test-verbose: ## Run tests with verbose output
	cargo test -- --test-threads=1 --nocapture

run: ## Run the server with sample data
	cargo run -- --entities examples/entities.json --events examples/events.json

run-ws: ## Run the server with WebSocket transport
	cargo run -- --ws 0.0.0.0:4000 --health-port 3000 --entities examples/entities.json --events examples/events.json

watch: ## Run with hot reload
	cargo watch -x 'run -- --entities examples/entities.json --events examples/events.json'

# Code Quality
lint: ## Run clippy linter
	cargo clippy --all-targets --all-features -- -D warnings

fmt: ## Format code
	cargo fmt --all

fmt-check: ## Check code formatting
	cargo fmt --all -- --check

audit: ## Security audit
	cargo audit

# Performance
bench: ## Run benchmarks
	cargo bench

# Docker
docker-build: ## Build Docker image
	docker build -t mcp-memory:latest .

docker-build-dev: ## Build development Docker image
	docker build --target development -t mcp-memory:dev .

docker-run: ## Run Docker container (STDIO)
	docker run -i --rm mcp-memory:latest

docker-run-ws: ## Run Docker container (WebSocket)
	docker run -p 4000:4000 -p 3000:3000 mcp-memory:latest --ws 0.0.0.0:4000 --health-port 3000

docker-security-scan: ## Run security scan on Docker image
	docker build --target security-scan -t mcp-memory:scan .
	docker run --rm mcp-memory:scan cat /tmp/trivy-report.sarif

# Docker Compose
compose-up: ## Start development environment
	docker-compose up -d

compose-dev: ## Start development environment with hot reload
	docker-compose up mcp-memory-dev

compose-down: ## Stop development environment
	docker-compose down

compose-logs: ## View logs
	docker-compose logs -f mcp-memory

# Kubernetes
k8s-deploy: ## Deploy to Kubernetes
	kubectl apply -f k8s/deployment.yaml

k8s-delete: ## Delete from Kubernetes
	kubectl delete -f k8s/deployment.yaml

k8s-status: ## Check Kubernetes status
	kubectl get pods -n mcp-memory
	kubectl get svc -n mcp-memory

k8s-logs: ## View Kubernetes logs
	kubectl logs -f deployment/mcp-memory -n mcp-memory

# Testing
test-integration: ## Run integration tests
	cargo test --test integration

test-load: ## Run load tests (requires wrk)
	@echo "Starting server in background..."
	@cargo run -- --ws 0.0.0.0:4000 --entities examples/entities.json &
	@sleep 3
	@echo "Running load tests..."
	@wrk -t12 -c400 -d30s http://localhost:4000/ || echo "Install wrk for load testing"
	@pkill -f mcp_memory_server

# Health Checks
health-check: ## Run health check
	curl -f http://localhost:3000/health/live

ready-check: ## Run readiness check
	curl -f http://localhost:3000/health/ready

metrics: ## View metrics
	curl http://localhost:3000/metrics

# Cleanup
clean: ## Clean build artifacts
	cargo clean

clean-docker: ## Clean Docker images
	docker rmi mcp-memory:latest mcp-memory:dev mcp-memory:scan || true
	docker system prune -f

# Setup
setup: ## Setup development environment
	rustup component add rustfmt clippy
	cargo install cargo-watch cargo-audit
	@echo "Development environment setup complete!"

# Documentation
docs: ## Generate and open documentation
	cargo doc --open

# Release
release-patch: ## Create a patch release
	@echo "Creating patch release..."
	@git tag $$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version | split(".") | .[0] + "." + .[1] + "." + (.[2] | tonumber + 1 | tostring)')

release-minor: ## Create a minor release
	@echo "Creating minor release..."
	@git tag $$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version | split(".") | .[0] + "." + (.[1] | tonumber + 1 | tostring) + ".0"')

# Quick start for new developers
quickstart: setup build test docker-build ## Complete setup for new developers
	@echo ""
	@echo "🚀 Quick start complete!"
	@echo ""
	@echo "Try these commands:"
	@echo "  make run          # Run with sample data"
	@echo "  make run-ws       # Run with WebSocket"
	@echo "  make compose-up   # Start full dev environment"
	@echo "  make health-check # Test health endpoint"
	@echo ""
