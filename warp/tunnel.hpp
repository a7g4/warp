#pragma once
#include "log.hpp"
#include "warp/error.hpp"
#include "warp/log.hpp"

#include <cstring>
#include <stdexcept>
#include <string>
#include <string_view>
#include <sys/socket.h>
#include <sys/un.h>

namespace warp {

class Tunnel {
public:
    Tunnel(std::string_view socket_path) : socket_path(socket_path) {
        if (socket_path.size() + 1 > sizeof(sockaddr_un::sun_path)) {
            throw warp::exception("Path of tunnel must be shorter than {}", sizeof(sockaddr_un::sun_path) - 1);
        }

        socket_fd = socket(AF_UNIX, SOCK_DGRAM, 0);
        if (socket_fd == -1) { throw warp::exception("Error calling socket(): {}", warp::logging::CError()); }

        sockaddr_un address;
        address.sun_family = AF_UNIX;
        std::strncpy(address.sun_path, socket_path.data(), sizeof(sockaddr_un::sun_path));

        if (bind(socket_fd, reinterpret_cast<sockaddr*>(&address), sizeof(address)) == -1) {
            throw warp::exception("Error calling bind(): {}", warp::logging::CError());
        }
        warp::log(INFO, "Warp tunnel ready at {}", socket_path);
    }

    ~Tunnel() {
        if (shutdown(socket_fd, SHUT_RDWR) < 0) { warp::log(ERROR, "Error calling shutdown(): {}", logging::CError()); }
        if (close(socket_fd) < 0) { warp::log(ERROR, "Error calling close(): {}", logging::CError()); }
        if (unlink(socket_path.data()) < 0) {
            warp::log(ERROR, "Error calling unlink({}): {}", socket_path, logging::CError());
        }
        warp::log(INFO, "warp::Tunnel at {} closed", socket_path);
    }

    Tunnel(const Tunnel&) = delete;
    Tunnel(Tunnel&&) = delete;
    Tunnel& operator=(const Tunnel&) = delete;
    Tunnel& operator=(Tunnel&&) = delete;

    int socket_fd;
    std::string socket_path;
};
} // namespace warp
