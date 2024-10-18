#pragma once

#include <string>

namespace warp {

class Transmitter {
    Transmitter(std::string remote_address, std::optional<std::string> bind_address) : remote_address(remote_address), bind_address(bind_address) {
    }

    std::string remote_address;
    std::optional<std::string> bind_address;
    int socket_fd;
};

}
