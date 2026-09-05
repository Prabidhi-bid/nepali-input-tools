# Builds the Fcitx5 addon and links it against the Rust cdylib.
#
#   cmake -B build -S cpp -DCMAKE_BUILD_TYPE=Release
#   cmake --build build
#
# The Rust library must exist first: cargo build -p xlit-fcitx5 --release.
# install.sh does both in order.

cmake_minimum_required(VERSION 3.16)
project(xlit-fcitx5 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

find_package(Fcitx5Core REQUIRED)
find_package(Fcitx5Utils REQUIRED)

# Where cargo put the cdylib. Overridable for a cross build or a debug profile.
set(XLIT_RUST_PROFILE "release" CACHE STRING "cargo profile the Rust library was built with")
set(XLIT_RUST_LIB "${CMAKE_CURRENT_SOURCE_DIR}/../../../target/${XLIT_RUST_PROFILE}/libxlit_fcitx5.so"
    CACHE FILEPATH "path to the Rust cdylib")

if(NOT EXISTS "${XLIT_RUST_LIB}")
    message(FATAL_ERROR
        "${XLIT_RUST_LIB} is missing.\n"
        "Build it first: cargo build -p xlit-fcitx5 --release")
endif()

add_library(xlit MODULE xlit-engine.cpp)
target_link_libraries(xlit PRIVATE Fcitx5::Core Fcitx5::Utils "${XLIT_RUST_LIB}")

# Fcitx5 loads addons by bare name, without the usual "lib" prefix.
set_target_properties(xlit PROPERTIES PREFIX "")

install(TARGETS xlit DESTINATION "${FCITX_INSTALL_LIBDIR}/fcitx5")
install(FILES "${CMAKE_CURRENT_SOURCE_DIR}/../data/xlit-addon.conf"
        RENAME xlit.conf
        DESTINATION "${FCITX_INSTALL_PKGDATADIR}/addon")
install(FILES "${CMAKE_CURRENT_SOURCE_DIR}/../data/xlit.conf"
        DESTINATION "${FCITX_INSTALL_PKGDATADIR}/inputmethod")
