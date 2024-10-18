#pragma once

#include <atomic>
#include <cstdint>
#include <linux/io_uring.h>
#include "warp/log.hpp"
#include "warp/error.hpp"
#include "warp/iour/actions.hpp"

namespace warp {

class SubmissionQueue {
public:
    bool needs_wakeup();
private:
    // I can't find any documentation saying that head and tail are the same type as the ring_mask, but the fact that
    // ring mask is used as a mask for head and tail is a strong indication that they are.
    using IndexType = decltype(io_sqring_offsets::ring_mask);
    // This should be enough to make it safe to use std::atomic on the raw IndexType?
    static_assert(sizeof(IndexType) == sizeof(std::atomic<IndexType>));

    std::atomic<IndexType>* head;
    std::atomic<IndexType>* tail;
    IndexType* ring_mask;

    // I don't think we actually care about this? We'll do all access through the mask
    IndexType* ring_entries;

    std::atomic<decltype(io_sqring_offsets::flags)>* flags;

    // TODO: Find out what this is and where it's documented
    decltype(io_sqring_offsets::dropped)* dropped;

    IndexType* array;

    io_uring_sqe* submission_queue_entries;
    friend class Iour;
};

class CompletionQueue {
    // I can't find any documentation saying that head and tail are the same type as the ring_mask, but the fact that
    // ring_mask is used as a mask for head and tail is a strong indication that they are.
    using IndexType = decltype(io_cqring_offsets::ring_mask);
    // This should be enough to make it safe to use std::atomic on the raw IndexType?
    static_assert(sizeof(IndexType) == sizeof(std::atomic<IndexType>));

    std::atomic<IndexType>* head;
    std::atomic<IndexType>* tail;
    IndexType* ring_mask;

    // I don't think we actually care about this? We'll do all access through the mask
    IndexType* ring_entries;

    // TODO: Find out what this is and where it's documented
    decltype(io_cqring_offsets::overflow)* overflow;

    io_uring_cqe* completion_queue_events;
    friend class Iour;
};

class Iour {
public:
    Iour(int queue_size);

    // The IOURingAction's lifetime must be at least the shorter of IOURing's lifetime or it's handle_completion method is called
    bool submit(const IOURingAction& action);
    bool submit(IOURingAction&&) = delete; // Disallow temporaries

    // Returns false if there was an error "starting" the queued actions OR if there was an interuption while waiting for completions (if requested)
    bool execute(bool wait_for_completions);
    size_t handle_completions(bool wait_for_completions);

private:
    int io_uring_fd;
    size_t to_submit;
    SubmissionQueue submission_queue;
    CompletionQueue completion_queue;
};

}
