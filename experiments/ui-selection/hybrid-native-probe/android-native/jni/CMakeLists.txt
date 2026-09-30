cmake_minimum_required(VERSION 3.22.1)
project(nr_jni C)

add_library(nr_jni SHARED nr_jni.c)
target_include_directories(nr_jni PRIVATE
    "${CMAKE_CURRENT_LIST_DIR}/../../../../shared/ffi/include"
)

# Supply the ABI-specific Rust static archive when configuring an Android build.
set(RUST_CORE_LIB "" CACHE FILEPATH "Path to librust_reading_core.a for this ABI")
if(RUST_CORE_LIB AND EXISTS "${RUST_CORE_LIB}")
    add_library(rust_reading_core STATIC IMPORTED)
    set_target_properties(rust_reading_core PROPERTIES IMPORTED_LOCATION "${RUST_CORE_LIB}")
    target_link_libraries(nr_jni PRIVATE rust_reading_core)
else()
    message(WARNING "RUST_CORE_LIB is unset; configure it before linking nr_jni")
endif()

target_link_libraries(nr_jni PRIVATE log m dl)
