#include "warp/iour/iour.hpp"
#include "iour.hpp"
#include <atomic>
#include <linux/io_uring.h>
#include <sys/poll.h>
#include <sys/syscall.h>
#include <cstring>
#include <poll.h>

namespace {
int io_uring_setup(unsigned entries, io_uring_params *p) {
    return (int) syscall(__NR_io_uring_setup, entries, p);
}
int io_uring_enter(int ring_fd, unsigned int to_submit, unsigned int min_complete, unsigned int flags) {
    return (int) syscall(__NR_io_uring_enter, ring_fd, to_submit, min_complete, flags, nullptr, 0);
}
}

namespace warp {

bool warp::SubmissionQueue::needs_wakeup() {
    return IORING_SQ_NEED_WAKEUP & flags->load(std::memory_order_acquire) != 0;
}

Iour::Iour(int queue_size) {
    io_uring_params params;
    std::memset(&params, 0, sizeof(params));
    params.flags = IORING_SETUP_COOP_TASKRUN | IORING_SETUP_TASKRUN_FLAG;
    io_uring_fd = io_uring_setup(queue_size, &params);
    if (io_uring_fd < 0) {
        throw warp::exception("Call to io_uring_setup() failed: {}", logging::CError());
    } else {
        warp::log(INFO, "io_uring fd = {}", io_uring_fd);
    }

    std::uint8_t* submission_queue_mmap = reinterpret_cast<std::uint8_t*>(mmap(0,
                                  params.sq_off.array + params.sq_entries * sizeof(SubmissionQueue::IndexType),
                                  PROT_WRITE | PROT_READ,
                                  MAP_SHARED_VALIDATE | MAP_POPULATE,
                                  io_uring_fd,
                                  IORING_OFF_SQ_RING
    ));

    if (submission_queue_mmap == MAP_FAILED) {
        throw warp::exception("Call to mmap() for submission queue failed: {}", logging::CError());
    }
    submission_queue.head = reinterpret_cast<std::atomic<SubmissionQueue::IndexType>*>(submission_queue_mmap + params.sq_off.head);
    submission_queue.tail = reinterpret_cast<std::atomic<SubmissionQueue::IndexType>*>(submission_queue_mmap + params.sq_off.tail);
    submission_queue.ring_mask = reinterpret_cast<SubmissionQueue::IndexType*>(submission_queue_mmap + params.sq_off.ring_mask);
    submission_queue.ring_entries = reinterpret_cast<SubmissionQueue::IndexType*>(submission_queue_mmap + params.sq_off.ring_entries);
    submission_queue.flags = reinterpret_cast<decltype(SubmissionQueue::flags)>(submission_queue_mmap + params.sq_off.flags);
    submission_queue.dropped = reinterpret_cast<decltype(SubmissionQueue::dropped)>(submission_queue_mmap + params.sq_off.dropped);
    submission_queue.array = reinterpret_cast<SubmissionQueue::IndexType*>(submission_queue_mmap + params.sq_off.array);

    void* submission_queue_entries = mmap(0,
                                  params.sq_entries * sizeof(io_uring_sqe),
                                  PROT_WRITE | PROT_READ,
                                  MAP_SHARED_VALIDATE | MAP_POPULATE,
                                  io_uring_fd,
                                  IORING_OFF_SQES
    );
    if (submission_queue_entries == MAP_FAILED) {
        throw warp::exception("Call to mmap() for submission queue entries failed: {}", logging::CError());
    }
    submission_queue.submission_queue_entries = reinterpret_cast<io_uring_sqe*>(submission_queue_entries);

    std::uint8_t* completion_queue_mmap = reinterpret_cast<std::uint8_t*>(mmap(0,
                                  params.cq_entries * sizeof(io_uring_cqe),
                                  PROT_WRITE | PROT_READ,
                                  MAP_SHARED_VALIDATE | MAP_POPULATE,
                                  io_uring_fd,
                                  IORING_OFF_CQ_RING
    ));
    if (completion_queue_mmap == MAP_FAILED) {
        throw warp::exception("Call to mmap() for completion queue failed: {}", logging::CError());
    }

    completion_queue.head = reinterpret_cast<std::atomic<CompletionQueue::IndexType>*>(completion_queue_mmap + params.cq_off.head);
    completion_queue.tail = reinterpret_cast<std::atomic<CompletionQueue::IndexType>*>(completion_queue_mmap + params.cq_off.tail);
    completion_queue.ring_mask = reinterpret_cast<CompletionQueue::IndexType*>(completion_queue_mmap + params.cq_off.ring_mask);
    completion_queue.ring_entries = reinterpret_cast<CompletionQueue::IndexType*>(completion_queue_mmap + params.cq_off.ring_entries);
    completion_queue.overflow = reinterpret_cast<decltype(CompletionQueue::overflow)>(completion_queue_mmap + params.cq_off.overflow);
    completion_queue.completion_queue_events = reinterpret_cast<io_uring_cqe*>(completion_queue_mmap + params.cq_off.cqes);
}

bool Iour::submit(const IOURingAction& action) {
    // Memory order acquire is for the kernel thread that may be writing to head
    SubmissionQueue::IndexType head = submission_queue.head->load(std::memory_order_acquire);

    // Memory order relaxed is because we should be the only one writing to tail
    SubmissionQueue::IndexType tail = submission_queue.tail->load(std::memory_order_relaxed);

    SubmissionQueue::IndexType mask = *submission_queue.ring_mask;

    if ((tail + 1) & mask == head & mask) {
        // Submission queue is full
        return false;
    }

    SubmissionQueue::IndexType slot = tail & mask;
    submission_queue.submission_queue_entries[slot] = action.generate_submission();
    submission_queue.submission_queue_entries[slot].user_data = reinterpret_cast<std::uintptr_t>(&action);
    submission_queue.array[slot] = slot;

    to_submit++;

    // Memory order release is to synchronise with the kernel thread that may be reading from tail
    submission_queue.tail->store(tail + 1, std::memory_order_release);
    return true;
}

bool Iour::execute(bool wait_for_completions) {
    if (io_uring_enter(io_uring_fd, to_submit, wait_for_completions ? 1 : 0, IORING_ENTER_GETEVENTS) < 0) {
        auto error = logging::CError();
        if (error.number() == EINTR) {
            // This is to be expected if we received a signal before the next completion event
        } else {
            warp::log(ERROR, "Error calling io_uring_enter(): {}", error);
        }
        return false;
    } else {
        to_submit = 0;
        return true;
    }
}

size_t Iour::handle_completions(bool wait_for_completions) {
    if (wait_for_completions) {
        execute(true);
    }

    SubmissionQueue::IndexType mask = *submission_queue.ring_mask;

    // Memory order acquire is for the kernel thread that may be writing to tail
    CompletionQueue::IndexType tail = completion_queue.tail->load(std::memory_order_acquire);

    // Memory order relaxed is because we should be the only one writing to head
    CompletionQueue::IndexType head = completion_queue.head->load(std::memory_order_relaxed);

    bool anything_requeued = false;

    size_t completions = 0;
    while ((head & mask) != (tail & mask)) {
        io_uring_cqe& completion_event = completion_queue.completion_queue_events[head & mask];
        IOURingAction* action = reinterpret_cast<IOURingAction*>(completion_event.user_data);
        action->handle_completion(completion_event);
        if (action->requeue()) {
            anything_requeued = true;
            submit(*action);
        }
        head++;
        completions++;
    }
    completion_queue.head->store(head, std::memory_order_release);

    if (anything_requeued) {
        execute(false);
    }
    return completions;
}

}
