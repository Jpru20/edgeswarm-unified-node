#include "swarm-funding-v176.h"
#include <cassert>
#include <iostream>

int main() {
    swarm_funding_meter_v176 m;
    assert(m.billing_chars() == 0);
    m.append("abc");
    assert(m.billing_chars() == 18);
    m.append("\n\"\\");
    assert(m.billing_chars() == 24);
    m.append(std::string(1, '\x01'));
    assert(m.billing_chars() == 30);
    auto copy = m;
    copy.append("\xc3\xa9");
    assert(copy.billing_chars() == 31);
    assert(m.billing_chars() == 30);
    swarm_funding_meter_v176 split, whole;
    const std::string utf8 = "\xc3\xa9\xe4\xb8\xad\xf0\x9f\x98\x80";
    whole.append(utf8);
    for (char c : utf8) split.append(std::string(1,c));
    assert(whole.billing_chars() == 19);
    assert(split.billing_chars() == whole.billing_chars());
    assert(split.raw_bytes == utf8.size());
    assert(split.tail.empty());
    swarm_funding_meter_v176 partial;
    partial.append("\xf0\x9f");
    assert(partial.billing_chars() == 17);
    partial.append("\x98\x80");
    assert(partial.billing_chars() == 17);
    assert(partial.raw_bytes == 4);
    assert(swarm_funding_chars_v176(100) == 400);
    assert(swarm_funding_id_valid_v176(std::string(32,'a')));
    assert(!swarm_funding_id_valid_v176(std::string(32,'g')));
    std::cout << "BILLING_METER_ASCII_ESCAPES_UTF16_SPLIT_UTF8=PASS\n";
}
