.PHONY: run

run:
	cargo run -p genos-camera

room:
	cargo run -p genos-camera -- --scene room.rhai
