#pragma once

#include <cstdint>
#include <limits>
#include <stdexcept>
#include <string>
#include <string_view>

// The financial meter is independent of the model's token-generation policy.
// Count the UTF-16 units of a JSON string wrapper (the existing billing basis),
// without keeping a second full copy of the generated text. An incomplete UTF-8
// suffix is conservatively counted until the next piece completes it.
struct swarm_funding_meter_v176 {
    uint64_t raw_bytes = 0;
    uint64_t escaped_chars = 0;
    std::string tail;

    void append(std::string_view piece) {
        if (piece.empty()) return;
        if (piece.size() > std::numeric_limits<uint64_t>::max() - raw_bytes) {
            throw std::overflow_error("funding_meter_overflow");
        }
        raw_bytes += piece.size();
        std::string s = std::move(tail);
        tail.clear();
        s.append(piece.data(), piece.size());
        size_t i = 0;
        while (i < s.size()) {
            const auto b = static_cast<unsigned char>(s[i]);
            if (b < 0x80) {
                if (b == '"' || b == '\\' || b == '\b' || b == '\t' ||
                    b == '\n' || b == '\f' || b == '\r') {
                    escaped_chars += 2;
                } else {
                    escaped_chars += b < 0x20 ? 6 : 1;
                }
                ++i;
                continue;
            }
            const size_t n = b >= 0xc2 && b <= 0xdf ? 2 :
                             b >= 0xe0 && b <= 0xef ? 3 :
                             b >= 0xf0 && b <= 0xf4 ? 4 : 0;
            if (n == 0) {
                ++escaped_chars;
                ++i;
                continue;
            }
            bool valid = true;
            for (size_t j = 1; j < n && i+j < s.size(); ++j) {
                const auto c = static_cast<unsigned char>(s[i+j]);
                if ((c & 0xc0) != 0x80 || (j == 1 &&
                    ((b == 0xe0 && c < 0xa0) || (b == 0xed && c >= 0xa0) ||
                     (b == 0xf0 && c < 0x90) || (b == 0xf4 && c >= 0x90)))) {
                    valid = false;
                    break;
                }
            }
            if (!valid) {
                // Treat each malformed byte as at most one replacement unit.
                ++escaped_chars;
                ++i;
                continue;
            }
            if (s.size()-i < n) {
                tail = s.substr(i);
                break;
            }
            escaped_chars += n == 4 ? 2 : 1;
            i += n;
        }
    }

    uint64_t billing_chars() const {
        return raw_bytes == 0 ? 0 : 15 + escaped_chars + tail.size();
    }
};

inline uint64_t swarm_funding_chars_v176(uint64_t billing_tokens) {
    if (billing_tokens > std::numeric_limits<uint64_t>::max() / 4) {
        throw std::overflow_error("funding_grant_overflow");
    }
    return billing_tokens * 4;
}

inline bool swarm_funding_id_valid_v176(const std::string & id) {
    if (id.size() != 32) return false;
    for (char c : id) {
        if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) return false;
    }
    return true;
}
