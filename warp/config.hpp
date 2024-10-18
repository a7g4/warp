#pragma once
#include <cstdint>
#include <cstring>
#include <stdexcept>
#include <string>
#include <string_view>
#include <vector>
#include <optional>
#include <sys/types.h>
#include <sys/socket.h>
#include <netdb.h>

#include "warp/error.hpp"
#include "warp/log.hpp"

namespace warp {

class TunnelConfig {
public:
    static std::optional<TunnelConfig> parse(std::string_view line) {
        return TunnelConfig(std::string(line));
    }
private:
    TunnelConfig(std::string&& name) : name(std::move(name)) {}
    std::string name;
};

class AddressPort {
public:
    static std::optional<AddressPort> parse(std::string_view line) {
        size_t separator = line.find_last_of(':');

        std::string_view address = line.substr(0, separator);
        // Handle case of wrapping IPv6 address in square brackets
        if (address.front() == '[' && address.back() == ']') {
            address = address.substr(1, address.size() - 2);
        }

        return AddressPort(address, separator == std::string_view::npos ? "" : line.substr(separator + 1));
    }

    std::string_view get_address() const {
        return address;
    }

    std::string_view get_port() const {
        return port;
    }

private:
    AddressPort(std::string_view address, std::string_view port) : address(std::string(address)), port(std::string(port)) { }
    std::string address;
    std::string port;
};

class InboundConfig {
public:
    static std::optional<InboundConfig> parse(std::string_view line) {
        auto address_port = AddressPort::parse(line);
        if (address_port) {
            return InboundConfig(*address_port);
        } else {
            warp::log(ERROR, "Failed to parse inbound config line: {}", line);
            return std::nullopt;
        }
    }

    std::string_view get_address() const {
        return address_port.get_address();
    }

    std::string_view get_port() const {
        return address_port.get_port();
    }

private:
    InboundConfig(const AddressPort& address_port) : address_port(address_port) { }
    AddressPort address_port;
};

class OutboundConfig {
public:
    static std::optional<OutboundConfig> parse(std::string_view line) {
        auto separator = line.find("=>");
        if (separator == std::string_view::npos) {
            return std::nullopt;
        }
        auto maybe_local = AddressPort::parse(line.substr(0, separator));
        auto maybe_remote = AddressPort::parse(line.substr(separator + 2));
        if (maybe_local && maybe_remote) {
            return OutboundConfig(*maybe_local, *maybe_remote);
        } else {
            return std::nullopt;
        }
    }
private:
    OutboundConfig(const AddressPort& local, const AddressPort& remote) : local(local), remote(remote) { }
    AddressPort local;
    AddressPort remote;
};

class GateConfig {
public:
    std::vector<TunnelConfig> tunnels;
    std::vector<InboundConfig> inbound;
    std::vector<OutboundConfig> outbound;

    // Dumb little parser that is good enough for now
    static GateConfig read_config(std::string_view config) {
        GateConfig gate_config;

        enum class ParserState {
            TUNNELS, INBOUND, OUTBOUND, UNKNOWN
        };

        ParserState parser_state = ParserState::UNKNOWN;

        size_t line_start = 0;
        size_t line_end = 0;

        while ((line_end = config.find('\n', line_start)) != std::string_view::npos) {
            std::string_view line = config.substr(line_start, line_end - line_start);
            line_start = line_end + 1;

            if (line.size() == 0) {
                continue;
            }

            // TODO: Trim whitespace
            // Ignore everything after a '#' character
            line = line.substr(0, line.find('#'));
            warp::log(INFO, "line = '{}', line_start = {}, line_end = {}", line, line_start, line_end);

            if (line == "[tunnels]") {
                parser_state = ParserState::TUNNELS;
                continue;
            } else if (line == "[inbound]") {
                parser_state = ParserState::INBOUND;
                continue;
            } else if (line == "[outbound]") {
                parser_state = ParserState::OUTBOUND;
                continue;
            }

            switch (parser_state) {
                case ParserState::TUNNELS: {
                    auto tunnel_config = TunnelConfig::parse(line);
                    if (tunnel_config) { gate_config.tunnels.push_back(*tunnel_config); }
                    continue;
                }
                case ParserState::INBOUND: {
                    auto inbound_config = InboundConfig::parse(line);
                    if (inbound_config) { gate_config.inbound.push_back(*inbound_config); }
                    continue;
                }
                case ParserState::OUTBOUND: {
                    auto outbound_config = OutboundConfig::parse(line);
                    if (outbound_config) { gate_config.outbound.push_back(*outbound_config); }
                    continue;
                }
                default:
                    warp::log(WARN, "Skipping line: {}", line);
            }
        }

        return gate_config;
    }
};

} // namespace warp
