#pragma once
#include <linux/io_uring.h>
#include <functional>
#include <sys/mman.h>
#include <cstdint>
#include <span>

namespace warp {
class IOURingAction {
public:
    IOURingAction() = default;
    virtual ~IOURingAction() = default;
    // io_uring_sqe::user_data will be overwritted by IOURing by a pointer to this instance of IOURingAction
    virtual io_uring_sqe generate_submission() const = 0;
    virtual void handle_completion(const io_uring_cqe& completion_event) = 0;
    virtual bool requeue() { return false; }
protected:
    IOURingAction(const IOURingAction&) = default;
    IOURingAction(IOURingAction&&) = default;
    IOURingAction& operator=(const IOURingAction&) = default;
    IOURingAction& operator=(IOURingAction&&) = default;
};

class ReadAction : public IOURingAction {
public:
    using Callback = std::function<void(const std::span<std::uint8_t>)>;
    ReadAction(int fd, size_t buffer_size, const Callback& callback) : fd(fd), buffer(buffer_size), callback(callback) { }
    ReadAction(int fd, size_t buffer_size, Callback&& callback) : fd(fd), buffer(buffer_size), callback(std::move(callback)) { }

    io_uring_sqe generate_submission() const override {
        io_uring_sqe event;
        std::memset(&event, 0, sizeof(event));
        event.opcode = IORING_OP_READ;
        event.fd = fd;
        event.addr = reinterpret_cast<std::uint64_t>(buffer.data());
        event.len = buffer.size();
        return event;
    }

    void handle_completion(const io_uring_cqe& completion_event) override {
        if (completion_event.res < 0) {
            warp::log(ERROR, "ReadAction action failed: {}", completion_event.res);
        } else {
            if (completion_event.res >= buffer.size()) {
                warp::log(WARN, "Buffer may not have been large enough for data");
            }
            callback(std::span(buffer.begin(), completion_event.res));
        }
    }

    void enable_requeue() {
        requeue_on_completion = true;
    }

    void disable_requeue() {
        requeue_on_completion = false;
    }

    bool requeue() override {
        return requeue_on_completion;
    }

    int fd;
    std::vector<std::uint8_t> buffer;
    Callback callback;
    bool requeue_on_completion = false;
};

class NoopAction : public IOURingAction {
    NoopAction(const std::function<void()>& callback) : callback(callback) { }
    NoopAction(std::function<void()>&& callback) : callback(std::move(callback)) { }

    io_uring_sqe generate_submission() const override {
        io_uring_sqe event;
        std::memset(&event, 0, sizeof(event));
        event.opcode = IORING_OP_NOP;
        return event;
    }

    void handle_completion(const io_uring_cqe& completion_event) override {
        if (completion_event.res < 0) {
            warp::log(ERROR, "NoopAction action failed: {}", completion_event.res);
        } else {
            callback();
        }
    }

    std::function<void()> callback;
};

}
