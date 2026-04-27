cmake_minimum_required(VERSION 3.22)
project(peko-stt-bin LANGUAGES CXX C)

# Phase 25 — build the upstream whisper.cpp `whisper-cli` tool for the
# OnePlus 6T. peko-stt (Rust) shells out to this binary; same-process
# FFI was tried first but cmake-rs fights NDK toolchain ABI selection
# and produces 32-bit ARM objects that won't link against aarch64.
# Building here uses our own NDK build script, mirroring the pattern
# that already works for peko-llm-daemon.

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(CMAKE_BUILD_TYPE STREQUAL "Release" OR NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
    add_compile_options(-O2 -fvisibility=hidden)
    if(NOT APPLE)
        add_link_options(-Wl,--gc-sections -Wl,-s)
    endif()
endif()

add_compile_options(-Wall -Wextra -Wno-unused-parameter -Wno-unused-function)

# Same Android pthread shim as peko-llm-daemon — see that CMakeLists
# for the rationale. NDK r27 + CMake 3.31 makes find_package(Threads)
# fail otherwise.
if(ANDROID)
    set(CMAKE_HAVE_PTHREAD_H          1 CACHE INTERNAL "Android bionic ships pthread.h")
    set(CMAKE_HAVE_LIBC_PTHREAD        1 CACHE INTERNAL "Android bionic libc has pthread_create")
    set(THREADS_PREFER_PTHREAD_FLAG  ON)
    set(CMAKE_THREAD_LIBS_INIT        "")
    set(CMAKE_HAVE_THREADS_LIBRARY     1)
    set(CMAKE_USE_WIN32_THREADS_INIT   0)
    set(CMAKE_USE_PTHREADS_INIT        1)
    set(Threads_FOUND               TRUE)
endif()

include(FetchContent)

# Build whisper.cpp's CLI as a subproject. WHISPER_BUILD_TESTS=OFF and
# WHISPER_BUILD_SERVER=OFF keep the build minimal; we only want the
# `whisper-cli` executable.
set(WHISPER_BUILD_TESTS    OFF CACHE BOOL "" FORCE)
set(WHISPER_BUILD_EXAMPLES ON  CACHE BOOL "" FORCE)
set(WHISPER_BUILD_SERVER   OFF CACHE BOOL "" FORCE)
set(BUILD_SHARED_LIBS      OFF CACHE BOOL "" FORCE)
set(GGML_NATIVE            OFF CACHE BOOL "" FORCE)
set(GGML_OPENMP            OFF CACHE BOOL "" FORCE)
set(GGML_METAL             OFF CACHE BOOL "" FORCE)

FetchContent_Declare(
    whisper
    GIT_REPOSITORY https://github.com/ggml-org/whisper.cpp.git
    GIT_TAG        v1.7.4
)
FetchContent_MakeAvailable(whisper)

# Upstream calls the CLI target `whisper-cli`. We do nothing else here —
# the install rule below copies it into the install prefix's bin/ for
# the build script to grab.
install(TARGETS whisper-cli RUNTIME DESTINATION bin)

message(STATUS "peko-stt-bin: configured for ${CMAKE_SYSTEM_NAME}")
