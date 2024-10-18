BUILD_DIR ?= build

CPPFLAGS += -std=c++23
CFLAGS += -I. -g

all: $(BUILD_DIR)/warp_gate $(BUILD_DIR)/sample_client

$(BUILD_DIR)/warp_gate: warp/main.cpp warp/log.hpp warp/tunnel.hpp warp/error.hpp warp/config.hpp $(BUILD_DIR)/iour.o $(BUILD_DIR)
	$(CXX) $(CPPFLAGS) $(CFLAGS) -o $@ warp/main.cpp $(BUILD_DIR)/iour.o

$(BUILD_DIR)/sample_client: warp/sample_client.cpp warp/log.hpp $(BUILD_DIR)
	$(CXX) $(CPPFLAGS) $(CFLAGS) -o $@ warp/sample_client.cpp

$(BUILD_DIR)/iour.o: warp/iour/iour.cpp warp/iour/iour.hpp
	$(CXX) $(CPPFLAGS) $(CFLAGS) -c -o $@ warp/iour/iour.cpp

$(BUILD_DIR): Makefile
	mkdir -p $(BUILD_DIR)
