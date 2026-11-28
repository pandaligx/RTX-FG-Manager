#pragma once

#include <algorithm>
#include <iterator>
#include <string_view>

namespace MenuZh
{
struct Entry
{
    std::string_view english;
    std::string_view chinese;
    bool formatSafe;
};

inline constexpr Entry entries[] = {
#include "MenuZhEntries.inc"
};

// Only visible text is translated. IDs, preference keys and input buffers retain
// the values supplied by OptiScaler. Every returned pointer has static lifetime.
inline std::string_view Find(std::string_view key, bool forFormat = false)
{
    const auto it = std::lower_bound(std::begin(entries), std::end(entries), key,
        [](const Entry& item, std::string_view value) { return item.english < value; });
    return it != std::end(entries) && it->english == key && (!forFormat || it->formatSafe)
        ? it->chinese : std::string_view{};
}

inline void Visible(const char*& begin, const char*& end, bool hideId = false)
{
    if (!begin)
        return;
    auto key = end ? std::string_view(begin, static_cast<size_t>(end - begin)) : std::string_view(begin);
    if (hideId)
        key = key.substr(0, key.find("##"));
    const auto translated = Find(key);
    if (!translated.empty())
    {
        begin = translated.data();
        end = begin + translated.size();
    }
}

inline const char* Format(const char* original)
{
    if (!original)
        return nullptr;
    const auto translated = Find(original, true);
    return translated.empty() ? original : translated.data();
}
}
