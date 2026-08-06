RECIPE_RUN_DEBUG_MCP := recipe-run-debug-mcp
INSTALL_DIR := $(HOME)/.local/bin

.PHONY: install
install:
	cargo build --release --workspace
	mkdir -p $(INSTALL_DIR)
# Copy to a temp name and rename over the target, rather than `cp` onto it. Overwriting a Mach-O
# binary in place reuses the same vnode, whose cached code signature no longer matches the new
# bytes, and macOS SIGKILLs the process at exec — "zsh: killed", with `codesign -v` still passing.
# rename(2) replaces the directory entry instead, so the poisoned vnode goes away with the old file.
	cp target/release/$(RECIPE_RUN_DEBUG_MCP) $(INSTALL_DIR)/$(RECIPE_RUN_DEBUG_MCP).new
	mv -f $(INSTALL_DIR)/$(RECIPE_RUN_DEBUG_MCP).new $(INSTALL_DIR)/$(RECIPE_RUN_DEBUG_MCP)
