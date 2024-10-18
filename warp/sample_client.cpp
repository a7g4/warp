#include "error.hpp"
#include "log.hpp"
#include "warp/error.hpp"
#include "warp/log.hpp"

#include <cerrno>
#include <chrono>
#include <cstring>
#include <fcntl.h>
#include <stdexcept>
#include <sys/socket.h>
#include <sys/un.h>
#include <thread>

namespace {
} // namespace

int main(int argc, char** argv) {

    std::string socket_path = "/tmp/warp";

    int socket_fd = socket(AF_UNIX, SOCK_DGRAM, 0);
    if (socket_fd == -1) {
        warp::log(INFO, "Error calling socket(): {}", warp::logging::CError());
        throw std::runtime_error("");
    }

    if (socket_path.size() + 1 > sizeof(sockaddr_un::sun_path)) {
        throw std::runtime_error("Use a shorter socket path");
    }

    sockaddr_un address;
    address.sun_family = AF_UNIX;
    std::strncpy(address.sun_path, socket_path.data(), sizeof(sockaddr_un::sun_path));

    if (connect(socket_fd, reinterpret_cast<sockaddr*>(&address), sizeof(address)) == -1) {
        throw warp::exception("Error calling connect(): {}", warp::logging::CError());
    }

    warp::log(INFO, "Connected to warp_gate at {}", socket_path);

    while (true) {
        // FIXME: Serialise this propperly
        auto now = std::chrono::utc_clock::now();
        if (write(socket_fd, &now, sizeof(now)) != sizeof(now)) {
            throw warp::exception("Error calling write(): {}", warp::logging::CError());
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }

    return 0;
}
