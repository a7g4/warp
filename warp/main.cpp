#include "config.hpp"
#include "receiver.hpp"
#include "warp/config.hpp"
#include "warp/receiver.hpp"
#include "warp/config.hpp"
#include "warp/iour/iour.hpp"
#include "warp/log.hpp"
#include "warp/tunnel.hpp"

#include <atomic>
#include <cerrno>
#include <cstddef>
#include <fcntl.h>
#include <signal.h>
#include <stdexcept>
#include <sys/socket.h>
#include <sys/uio.h>
#include <sys/un.h>
#include <unistd.h>
#include <fstream>
#include <ranges>

namespace {
std::atomic<bool> keep_running = true;

void signal_handler(int) { keep_running.store(false, std::memory_order_relaxed); }

} // namespace

int main(int argc, char** argv) {
    signal(SIGINT, &signal_handler);

    std::ifstream input("config");
    warp::GateConfig config = warp::GateConfig::read_config(std::string((std::istreambuf_iterator<char>(input)),
                                                                    std::istreambuf_iterator<char>()));

    std::vector<warp::Receiver> receivers = config.inbound
           | std::ranges::views::transform([](auto inbound) { return warp::Receiver::construct(inbound.get_address(), inbound.get_port()); })
           | std::views::join
           | std::ranges::to<std::vector>();

    warp::Tunnel tunnel("/tmp/warp");

    int receive_buffer_size = 0;
    socklen_t sizeof_receive_buffer_size = sizeof(receive_buffer_size);
    if (getsockopt(tunnel.socket_fd, SOL_SOCKET, SO_RCVBUF, &receive_buffer_size, &sizeof_receive_buffer_size) == -1) {
        warp::log(INFO, "Error calling getsockopt(): {}", warp::logging::CError());
        throw std::runtime_error("");
    } else {
        warp::log(INFO, "Receive buffer size is {} bytes", receive_buffer_size);
    }

    warp::Iour io_uring(63);
    warp::ReadAction action = warp::ReadAction(tunnel.socket_fd, receive_buffer_size, [](std::span<std::uint8_t> data) {
        warp::log(INFO,
                  "Received latency = {}",
                  std::chrono::utc_clock::now() - *reinterpret_cast<std::chrono::utc_clock::time_point*>(data.data()));
    });
    action.enable_requeue();
    io_uring.submit(action);
    io_uring.execute(false);
    while (keep_running.load(std::memory_order_relaxed)) {
        size_t completions = io_uring.handle_completions(true);
    }

    return 0;
}
