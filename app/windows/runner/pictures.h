#ifndef RUNNER_PICTURES_H_
#define RUNNER_PICTURES_H_

#include <cstddef>
#include <cstdint>
#include <vector>

// Pictures on the Windows clipboard come as the system's own bitmaps, which
// other systems do not read, and many Windows programs paste nothing else.
// These turn pictures into PNG and back with Windows' own decoders (WIC), so
// that nothing written here reads bytes that came from other programs or
// other devices. The bitmap headers are put together in the core.
namespace pictures {

// The most a copied picture may take, as on the Mac.
constexpr size_t kMaxBytes = 24 * 1024 * 1024;

// Larger pictures are not unpacked, so that one copy cannot use up memory.
constexpr uint64_t kMaxPixels = 40ull * 1000 * 1000;

// What decoding found.
enum class Decoded { kPicture, kNotAPicture, kTooLarge };

// Any picture Windows can read, such as PNG, JPEG or BMP, as rows of blue,
// green, red and alpha bytes from the top down.
Decoded Decode(const uint8_t* bytes, size_t size, std::vector<uint8_t>& pixels,
               uint32_t& width, uint32_t& height);

// Those rows as a fresh PNG, or nothing if Windows could not write it.
std::vector<uint8_t> EncodePng(const std::vector<uint8_t>& pixels,
                               uint32_t width, uint32_t height);

}  // namespace pictures

#endif  // RUNNER_PICTURES_H_
