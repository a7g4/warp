#pragma once

#include <cerrno>
#include <chrono>
#include <cstring>
#include <format>
#include <iostream>
#include <source_location>
#include <stdexcept>
#include <string>
#include <string_view>

#define INFO warp::logging::Info()
#define WARN warp::logging::Warn()
#define ERROR warp::logging::Error()

namespace warp {

namespace logging {

class Tag {
public:
    std::ostream& ostream() const { throw std::runtime_error("warp::logging::Tag should not be instantiated"); };
    std::source_location source_location() const { return source; }
    constexpr std::string_view tag_string() const { return "UNKNOWN"; }

protected:
    Tag() = delete;
    Tag(std::source_location source) : source(source) { }
    const std::source_location source;
};

struct Info : Tag {
    Info(std::source_location source = std::source_location::current()) : Tag(source) { }
    std::ostream& ostream() const { return std::cout; }
    constexpr std::string_view tag_string() const { return " INFO"; }
};

struct Warn : Tag {
    Warn(std::source_location source = std::source_location::current()) : Tag(source) { }
    std::ostream& ostream() const { return std::cerr; }
    constexpr std::string_view tag_string() const { return " WARN"; }
};

struct Error : Tag {
    Error(std::source_location source = std::source_location::current()) : Tag(source) { }
    std::ostream& ostream() const { return std::cerr; }
    constexpr std::string_view tag_string() const { return " ERROR"; }
};

class CError {
public:
    using errno_t = decltype(errno);
    CError() : error_number_(errno) {};
    CError(errno_t error_number) : error_number_(error_number) {};

    errno_t number() const {
        return error_number_;
    }

    std::string_view description() const {
        return std::strerror(error_number_);
    }

private:
    const errno_t error_number_;
    friend class std::formatter<CError, char>;
};

} // namespace logging

template <typename Tag, typename... Args>
void log(Tag log_tag, std::string_view format, Args&&... args) {
    const std::source_location& location = log_tag.source_location();

    constexpr int PREFIX_LENGTH = 70;
    constexpr std::string_view separator(": ");
    const std::string new_line_spacer(PREFIX_LENGTH, ' ');

    const std::string time =
        std::format("{:%FT%TZ} ", std::chrono::time_point_cast<std::chrono::milliseconds>(std::chrono::utc_clock::now()));
    const std::string line_col = std::format(":{}:{}", location.line(), location.column());
    const int file_name_length = strlen(location.file_name());

    // We'll use negative truncation to represent the ammount of padding needed
    const int truncation_needed = file_name_length + time.size() + line_col.size() + log_tag.tag_string().size()
                                + separator.size() - PREFIX_LENGTH;

    std::string prefix;
    prefix.reserve(PREFIX_LENGTH);
    if (truncation_needed > 0) {
        std::string_view file_name_substring(location.file_name() + std::max(0, truncation_needed));
        prefix = std::format("{}{}{}{}{}", time, file_name_substring, line_col, log_tag.tag_string(), separator);
        prefix[time.size() + 0] = '.';
        prefix[time.size() + 1] = '.';
        prefix[time.size() + 2] = '.';
    } else {
        prefix = std::format("{}{}{}", time, location.file_name(), line_col);
        prefix.append(-truncation_needed, ' ');
        prefix.append(log_tag.tag_string());
        prefix.append(separator);
    }

    std::string payload = std::vformat(format, std::make_format_args(args...));
    std::string formatted;
    size_t line_start = 0, line_length = std::string::npos;
    do {
        formatted.append(line_start == 0 ? prefix : new_line_spacer);
        line_length = payload.find('\n', line_start);
        formatted.append(payload, line_start, line_length);
        formatted.push_back('\n');
        line_start += line_length + 1;
    } while (line_length != std::string::npos);
    log_tag.ostream() << formatted;
}

} // namespace warp

template <>
struct std::formatter<warp::logging::CError, char> {
    template <class ParseContext>
    constexpr ParseContext::iterator parse(ParseContext& ctx) {
        const auto* begin = ctx.begin();
        const auto* end = ctx.end();
        const auto* end_brace = std::find(begin, end, '}');
        if (end_brace == end) { throw std::format_error("Invalid format - No matching brace"); }

        // TODO: Do something useful with this
        auto format_string = std::string_view(begin, end_brace);
        return end_brace;
    }

    template <class FormatContext>
    auto format(warp::logging::CError cerror, FormatContext& ctx) const {
        std::string_view error_description = cerror.description();
        for (const auto& c : error_description) {
            *ctx.out() = c;
            ctx.out()++;
        }
        return ctx.out();
    }
};
