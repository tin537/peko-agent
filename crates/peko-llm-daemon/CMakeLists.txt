cmake_minimum_required(VERSION 3.22)
project(peko-llm-daemon LANGUAGES CXX C)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# ── Option: Vulkan backend for Android (Adreno/Mali GPU acceleration) ───────
# Default ON for Android target, OFF for host (macOS doesn't have Vulkan out of the box).
if(ANDROID)
    option(PEKO_VULKAN "Enable llama.cpp Vulkan backend (Adreno/Mali GPU)" ON)
else()
    option(PEKO_VULKAN "Enable llama.cpp Vulkan backend" OFF)
endif()

# ── Release optimizations ──────────────────────────────────────────────────
if(CMAKE_BUILD_TYPE STREQUAL "Release" OR NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release)
    add_compile_options(-O2 -fvisibility=hidden)
    if(NOT APPLE)
        add_link_options(-Wl,--gc-sections -Wl,-s)
    endif()
endif()

add_compile_options(-Wall -Wextra -Wpedantic -Wno-unused-parameter -Wno-unused-function)

# ── Fetch llama.cpp (pinned tag) ───────────────────────────────────────────
# Using CMake FetchContent keeps the project a single clone — no submodule dance.
include(FetchContent)

# llama.cpp options BEFORE FetchContent_MakeAvailable so they're picked up
set(LLAMA_BUILD_TESTS    OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
set(LLAMA_BUILD_SERVER   OFF CACHE BOOL "" FORCE)
set(LLAMA_CURL           OFF CACHE BOOL "" FORCE)
set(BUILD_SHARED_LIBS    OFF CACHE BOOL "" FORCE)

# Vulkan backend toggle
if(PEKO_VULKAN)
    set(GGML_VULKAN ON CACHE BOOL "" FORCE)
    message(STATUS "peko-llm-daemon: Vulkan backend ENABLED (Adreno/Mali GPU)")
else()
    set(GGML_VULKAN OFF CACHE BOOL "" FORCE)
    message(STATUS "peko-llm-daemon: Vulkan DISABLED — CPU-only inference")
endif()

# CPU-side ggml tuning
set(GGML_NATIVE       OFF CACHE BOOL "" FORCE)  # not for cross-compile
set(GGML_OPENMP       OFF CACHE BOOL "" FORCE)  # Android NDK lacks OpenMP by default

# Pin a known-good tag. Update periodically.
# The repo moved to ggml-org/ and adopted master-<sha7> tag format.
FetchContent_Declare(
    llama
    GIT_REPOSITORY https://github.com/ggml-org/llama.cpp.git
    GIT_TAG        268d61e178f5de67b0f5eaed3bc84b1e6daccf96  # current master 2026-04-17
    # GIT_SHALLOW can't be used with a specific commit SHA — must be a branch/tag.
)
FetchContent_MakeAvailable(llama)

# cpp-httplib needs threads
find_package(Threads REQUIRED)

# ── Our daemon binary ──────────────────────────────────────────────────────
add_executable(peko-llm-daemon
    src/main.cpp
    src/http_server.cpp
    src/llm_session.cpp
    src/chat_template.cpp
    src/openai_api.cpp
)

target_include_directories(peko-llm-daemon PRIVATE
    src
    third_party
)

target_link_libraries(peko-llm-daemon PRIVATE
    Threads::Threads
    llama                 # llama.cpp inference (from FetchContent)
    ggml                  # ggml tensor backend (CPU + Vulkan if enabled)
)

# Propagate feature flags into our code
if(PEKO_VULKAN)
    target_compile_definitions(peko-llm-daemon PRIVATE PEKO_VULKAN=1)
endif()

# Android-specific linkage
if(ANDROID)
    target_link_libraries(peko-llm-daemon PRIVATE log android)
endif()

if(CMAKE_SYSTEM_NAME STREQUAL "Android" OR ANDROID)
    set_target_properties(peko-llm-daemon PROPERTIES
        LINK_FLAGS "-pie -fPIE"
    )
endif()

install(TARGETS peko-llm-daemon RUNTIME DESTINATION bin)

message(STATUS "peko-llm-daemon configured for ${CMAKE_SYSTEM_NAME} (${CMAKE_SYSTEM_PROCESSOR}), Vulkan=${PEKO_VULKAN}")
