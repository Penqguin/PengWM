# Makefile for pengwm - BSP Window Manager for macOS (fixed layout)

# ---- Toolchain ----
CC      ?= clang
CFLAGS  ?= -Wall -Wextra -Werror -std=c99 -pedantic -O2
DEBUG_CFLAGS ?= -Wall -Wextra -Werror -std=c99 -pedantic -g -DDEBUG -O0
CPPFLAGS ?=
LDFLAGS  ?=
LDLIBS   ?= -framework ApplicationServices -framework CoreFoundation -framework CoreGraphics

# ---- Directories ----
SRCDIR   ?= src
OBJDIR   ?= build
OUTDIR   ?= bin

# ---- Install dirs ----
PREFIX   ?= /usr/local
DESTDIR  ?=
INSTALL_BINDIR ?= $(DESTDIR)$(PREFIX)/bin

# ---- Sources / Objects ----
SOURCES  := main.c cli.c window_manager.c window.c window_control.c bsp.c display.c config.c utils.c
SOURCES  := $(addprefix $(SRCDIR)/,$(SOURCES))
OBJECTS  := $(patsubst $(SRCDIR)/%.c,$(OBJDIR)/%.o,$(SOURCES))

TARGET        := $(OUTDIR)/pengwm
DEBUG_TARGET  := $(OUTDIR)/pengwm-debug

# ---- Phony targets ----
.PHONY: all debug install uninstall clean format analyze test dist setup help

all: $(TARGET)

debug: CFLAGS := $(DEBUG_CFLAGS)
debug: $(DEBUG_TARGET)

# ---- Directories ----
$(OBJDIR):
	@mkdir -p $(OBJDIR)

$(OUTDIR):
	@mkdir -p $(OUTDIR)

# ---- Link ----
$(TARGET): $(OBJECTS) | $(OUTDIR)
	@echo "Linking $@"
	@$(CC) $(CFLAGS) $(CPPFLAGS) $(LDFLAGS) -o $@ $(OBJECTS) $(LDLIBS)
	@echo "Built pengwm successfully"

$(DEBUG_TARGET): $(OBJECTS) | $(OUTDIR)
	@echo "Linking debug build $@"
	@$(CC) $(CFLAGS) $(CPPFLAGS) $(LDFLAGS) -o $@ $(OBJECTS) $(LDLIBS)
	@echo "Built pengwm debug successfully"

# ---- Compile ----
$(OBJDIR)/%.o: $(SRCDIR)/%.c | $(OBJDIR)
	@echo "Compiling $<"
	@$(CC) $(CFLAGS) $(CPPFLAGS) -MMD -MP -c $< -o $@

-include $(OBJECTS:.o=.d)

# ---- Install / Uninstall (macOS) ----
install: $(TARGET)
	@echo "Installing pengwm to $(INSTALL_BINDIR)"
	@mkdir -p "$(INSTALL_BINDIR)"
	@cp "$(TARGET)" "$(INSTALL_BINDIR)/pengwm"
	@echo "Code signing pengwm (best-effort)..."
	@codesign -s - "$(INSTALL_BINDIR)/pengwm" || echo "Warning: Code signing failed"
	@echo "Installation complete"
	@echo ""
	@echo "IMPORTANT: Grant Accessibility permission to pengwm:"
	@echo "System Settings > Privacy & Security > Accessibility"

uninstall:
	@echo "Removing pengwm from $(INSTALL_BINDIR)"
	@rm -f "$(INSTALL_BINDIR)/pengwm"
	@echo "Uninstallation complete"

# ---- Housekeeping ----
clean:
	@echo "Cleaning build files"
	@rm -rf "$(OBJDIR)" "$(OUTDIR)"

format:
	@echo "Formatting code with clang-format"
	@clang-format -i -style="{BasedOnStyle: LLVM, IndentWidth: 4, ColumnLimit: 100}" $(SRCDIR)/*.c $(SRCDIR)/*.h

analyze: clean
	@echo "Running static analysis"
	@scan-build -o analysis-results $(MAKE) all

test: debug
	@echo "Running tests"
	@echo "(no tests yet)"

dist: clean
	@echo "Creating distribution archive"
	@mkdir -p dist
	@tar -czf dist/pengwm.tar.gz --exclude-vcs --exclude='__MACOSX' --exclude='*.DS_Store' .
	@echo "Distribution created at dist/pengwm.tar.gz"

setup:
	@echo "Setting up development environment (clang-format, scan-build, etc.)"
	@echo "(manual step on macOS: xcode-select --install)"

help:
	@echo "pengwm Makefile - common targets:"
	@echo ""
	@echo "Build:"
	@echo "  make            Build release binary -> $(TARGET)"
	@echo "  make debug      Build debug binary   -> $(DEBUG_TARGET)"
	@echo "  make clean      Remove build files"
	@echo ""
	@echo "Installation:"
	@echo "  make install    Install to \$$DESTDIR\$$PREFIX/bin (defaults to /usr/local/bin)"
	@echo "  make uninstall  Remove installed binary"
	@echo ""
	@echo "Development:"
	@echo "  make format     Format code with clang-format"
	@echo "  make analyze    Run static analysis"
	@echo "  make test       Run test suite (placeholder)"
	@echo "  make setup      Setup development environment"
	@echo ""
	@echo "Distribution:"
	@echo "  make dist       Create distribution package"
