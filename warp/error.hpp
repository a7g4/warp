#pragma once

#include <format>
#include <stdexcept>

namespace warp {

template <typename... Args>
std::runtime_error exception(std::string_view format, Args&&... args) {
    return std::runtime_error(std::vformat(format, std::make_format_args(args...)));
}

template <typename... Args>
std::runtime_error exception(std::exception* cause, std::string_view format, Args&&... args) {
    return std::runtime_error(std::format(format, std::make_format_args(args...))
                              + std::format("\nCaused by: {}", cause->what()));
}

} // namespace warp
