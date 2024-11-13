-:
	@cargo b -q 2>/dev/null;
server:
	@RUST_LOG="trace,test::foo=info,test::foo::bar=debug" ./target/debug/gnostr-chat-server
chat:
	RUST_LOG="trace,test::foo=info,test::foo::bar=debug" ./target/debug/gnostr-chat
install:
	cargo install --bins --path .
