#include "pictures.h"

#include <windows.h>

#include <objbase.h>
#include <wincodec.h>
#include <wrl/client.h>

#include <cstring>

using Microsoft::WRL::ComPtr;

namespace pictures {

namespace {

ComPtr<IWICImagingFactory> Factory() {
  ComPtr<IWICImagingFactory> factory;
  ::CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                     IID_PPV_ARGS(&factory));
  return factory;
}

}  // namespace

Decoded Decode(const uint8_t* bytes, size_t size, std::vector<uint8_t>& pixels,
               uint32_t& width, uint32_t& height) {
  pixels.clear();
  if (bytes == nullptr || size == 0 || size > kMaxBytes) {
    return Decoded::kNotAPicture;
  }
  ComPtr<IWICImagingFactory> factory = Factory();
  ComPtr<IWICStream> stream;
  ComPtr<IWICBitmapDecoder> decoder;
  ComPtr<IWICBitmapFrameDecode> frame;
  if (!factory || FAILED(factory->CreateStream(&stream)) ||
      FAILED(stream->InitializeFromMemory(const_cast<BYTE*>(bytes),
                                          static_cast<DWORD>(size))) ||
      FAILED(factory->CreateDecoderFromStream(
          stream.Get(), nullptr, WICDecodeMetadataCacheOnDemand, &decoder)) ||
      FAILED(decoder->GetFrame(0, &frame))) {
    return Decoded::kNotAPicture;
  }
  UINT frame_width = 0;
  UINT frame_height = 0;
  if (FAILED(frame->GetSize(&frame_width, &frame_height)) ||
      frame_width == 0 || frame_height == 0) {
    return Decoded::kNotAPicture;
  }
  if (static_cast<uint64_t>(frame_width) * frame_height > kMaxPixels) {
    return Decoded::kTooLarge;
  }
  ComPtr<IWICBitmapSource> converted;
  if (FAILED(::WICConvertBitmapSource(GUID_WICPixelFormat32bppBGRA,
                                      frame.Get(), &converted))) {
    return Decoded::kNotAPicture;
  }
  const UINT stride = frame_width * 4;
  pixels.resize(static_cast<size_t>(stride) * frame_height);
  if (FAILED(converted->CopyPixels(nullptr, stride,
                                   static_cast<UINT>(pixels.size()),
                                   pixels.data()))) {
    pixels.clear();
    return Decoded::kNotAPicture;
  }
  width = frame_width;
  height = frame_height;
  return Decoded::kPicture;
}

std::vector<uint8_t> EncodePng(const std::vector<uint8_t>& pixels,
                               uint32_t width, uint32_t height) {
  if (width == 0 || height == 0 ||
      pixels.size() != static_cast<size_t>(width) * height * 4) {
    return {};
  }
  ComPtr<IWICImagingFactory> factory = Factory();
  ComPtr<IStream> stream;
  ComPtr<IWICBitmapEncoder> encoder;
  ComPtr<IWICBitmapFrameEncode> frame;
  ComPtr<IPropertyBag2> options;
  if (!factory || FAILED(::CreateStreamOnHGlobal(nullptr, TRUE, &stream)) ||
      FAILED(factory->CreateEncoder(GUID_ContainerFormatPng, nullptr,
                                    &encoder)) ||
      FAILED(encoder->Initialize(stream.Get(), WICBitmapEncoderNoCache)) ||
      FAILED(encoder->CreateNewFrame(&frame, &options)) ||
      FAILED(frame->Initialize(options.Get())) ||
      FAILED(frame->SetSize(width, height))) {
    return {};
  }
  WICPixelFormatGUID format = GUID_WICPixelFormat32bppBGRA;
  if (FAILED(frame->SetPixelFormat(&format)) ||
      format != GUID_WICPixelFormat32bppBGRA ||
      FAILED(frame->WritePixels(height, width * 4,
                                static_cast<UINT>(pixels.size()),
                                const_cast<BYTE*>(pixels.data()))) ||
      FAILED(frame->Commit()) || FAILED(encoder->Commit())) {
    return {};
  }
  STATSTG stat = {};
  HGLOBAL global = nullptr;
  if (FAILED(stream->Stat(&stat, STATFLAG_NONAME)) ||
      FAILED(::GetHGlobalFromStream(stream.Get(), &global))) {
    return {};
  }
  const size_t size = static_cast<size_t>(stat.cbSize.QuadPart);
  if (size == 0 || size > kMaxBytes) {
    return {};
  }
  const void* data = ::GlobalLock(global);
  if (data == nullptr) {
    return {};
  }
  std::vector<uint8_t> png(size);
  std::memcpy(png.data(), data, size);
  ::GlobalUnlock(global);
  return png;
}

}  // namespace pictures
