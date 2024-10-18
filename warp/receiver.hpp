#pragma once
#include "error.hpp"
#include "warp/log.hpp"
#include "warp/error.hpp"
#include <string_view>
#include <unistd.h>
#include <sys/socket.h>
#include <generator>

// Why do I need these?
#include <netinet/in.h>
#include <netdb.h>

namespace warp {
class Receiver {
public:
    /// @param address_string an IPv4 or IPv6 address in the form of "123.123.123.123" or "2001:db8::1"
    /// @param port_string    a port number (or if you're feeling fancy, a service name like "ftp")
    static std::generator<Receiver&&> construct(std::string_view address_string, std::string_view port_string) {
        addrinfo inbound_address;
        addrinfo hints;
        std::memset(&hints, 0, sizeof(hints));
        hints.ai_family = AF_UNSPEC;
        hints.ai_socktype = SOCK_DGRAM;
        hints.ai_flags = AI_PASSIVE;

        addrinfo* candidates;
        auto result = getaddrinfo(address_string.data(), port_string.data(), &hints, &candidates);
        if (result != 0) {
            throw warp::exception("Error calling getaddrinfo(): {}", gai_strerror(result));
        }

        size_t n_candidates = 0;
        while (candidates != nullptr) {
            n_candidates++;

            int socket_fd = socket(candidates->ai_family, candidates->ai_socktype, candidates->ai_protocol);
            if (socket_fd < 0) {
                throw warp::exception("Call to socket() failed: {}", logging::CError());
            }

            if (candidates->ai_family == AF_INET6) {
                // To avoid setting up a dual-stack socket and potentially dealing with mapped IPV4 address, set the
                // all IPV6 sockets to IPV6 only.
                constexpr int ENABLE_IPV6_ONLY = 1;
                setsockopt(socket_fd, IPPROTO_IPV6, IPV6_V6ONLY, (void *)&ENABLE_IPV6_ONLY, sizeof(ENABLE_IPV6_ONLY));
            }

            int bind_result = bind(socket_fd, candidates->ai_addr, candidates->ai_addrlen);
            if (bind_result < 0) {
                throw warp::exception("Call to bind({} : {}, candidate {}) failed: {}", address_string, port_string, n_candidates, logging::CError());
            }

            co_yield Receiver(socket_fd);

            candidates = candidates->ai_next;
        }
    }

    Receiver(int socket_fd) : socket_fd(socket_fd) { }

    ~Receiver() {
        if (socket_fd == INVALID_FD) { return; }
        if (close(socket_fd) < 0) {
            warp::log(ERROR, "Error calling close({}): {}", socket_fd, logging::CError());
        }
    }

    Receiver(const Receiver&) = delete;
    Receiver(Receiver&& other) : socket_fd(other.socket_fd) {
        other.socket_fd = -1;
    }
    Receiver& operator=(const Receiver&) = delete;
    Receiver& operator=(Receiver&& other) {
        if (socket_fd != INVALID_FD) {
            this->~Receiver();
        }
        socket_fd = other.socket_fd;
        other.socket_fd = INVALID_FD;
        return *this;
    }

    static constexpr int INVALID_FD = -1;
    int socket_fd;
};
}
