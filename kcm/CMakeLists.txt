# SPDX-License-Identifier: GPL-2.0-or-later
# "Clavier Apple" module of System Settings (#250).
#
#   cmake -S kcm -B build -DCMAKE_INSTALL_PREFIX=/usr -DCMAKE_BUILD_TYPE=Release
#   cmake --build build && DESTDIR=... cmake --install build
#
# Installs:
#   lib/qt6/plugins/plasma/kcms/systemsettings/kcm_applekeyboard.so  (QML inside)
#   share/applications/kcm_applekeyboard.desktop                     (generated)
#   share/locale/fr/LC_MESSAGES/kcm_applekeyboard.mo
cmake_minimum_required(VERSION 3.20)
project(kcm_applekeyboard LANGUAGES CXX)

set(QT_MIN_VERSION 6.6)
set(KF_MIN_VERSION 6.0)

find_package(ECM ${KF_MIN_VERSION} REQUIRED NO_MODULE)
set(CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)
include(ECMFindQmlModule)

find_package(Qt6 ${QT_MIN_VERSION} REQUIRED COMPONENTS Core Gui DBus Qml Quick Concurrent)
find_package(KF6 ${KF_MIN_VERSION} REQUIRED COMPONENTS KCMUtils I18n CoreAddons)
ecm_find_qmlmodule(org.kde.kirigami 2.20)
ecm_find_qmlmodule(org.kde.kcmutils 1.0)

add_definitions(-DTRANSLATION_DOMAIN=\"kcm_applekeyboard\")

kcmutils_add_qml_kcm(kcm_applekeyboard SOURCES src/kcm.cpp src/bridge.cpp src/bridge.h src/devicename.cpp src/devicename.h)
target_link_libraries(kcm_applekeyboard PRIVATE
    Qt6::Core Qt6::Gui Qt6::DBus Qt6::Qml Qt6::Quick Qt6::Concurrent
    KF6::KCMUtilsQuick KF6::I18n KF6::CoreAddons)

# Unit test of the device-name command (#248): the name never goes through a
# shell. `ctest --test-dir build` (BUILD_TESTING, on by default with ECM).
if(BUILD_TESTING)
    find_package(Qt6 ${QT_MIN_VERSION} REQUIRED COMPONENTS Test)
    add_executable(test_devicename tests/test_devicename.cpp src/devicename.cpp src/devicename.h)
    target_link_libraries(test_devicename PRIVATE Qt6::Core Qt6::Test)
    add_test(NAME kcm_devicename COMMAND test_devicename)

    # The real module on a sealed bench (audit of 2026-10-03): loaded with
    # KCModuleLoader as System Settings does, private session bus without any
    # activation directory, temporary HOME/XDG, fake akmctl (tests/private_bus.sh).
    find_package(Qt6 ${QT_MIN_VERSION} REQUIRED COMPONENTS Widgets QuickWidgets)
    find_package(KF6 ${KF_MIN_VERSION} REQUIRED COMPONENTS KCMUtils)
    find_program(DBUS_RUN_SESSION dbus-run-session)
    if(DBUS_RUN_SESSION)
        add_executable(test_kcm_module tests/test_kcm_module.cpp)
        target_link_libraries(test_kcm_module PRIVATE
            Qt6::Core Qt6::Gui Qt6::Widgets Qt6::Quick Qt6::QuickWidgets Qt6::Test
            KF6::KCMUtils KF6::I18n KF6::CoreAddons)
        target_compile_definitions(test_kcm_module PRIVATE
            KCM_PLUGIN_FILE="$<TARGET_FILE:kcm_applekeyboard>"
            KCM_GMO_FILE="${CMAKE_CURRENT_BINARY_DIR}/kcm_applekeyboard.gmo")
        add_dependencies(test_kcm_module kcm_applekeyboard)
        add_test(NAME kcm_module COMMAND bash ${CMAKE_CURRENT_SOURCE_DIR}/tests/private_bus.sh $<TARGET_FILE:test_kcm_module>)
        set_tests_properties(kcm_module PROPERTIES TIMEOUT 180 SKIP_RETURN_CODE 77)
    endif()
endif()

# Translations: po/<lang>/kcm_applekeyboard.po -> <lang>/LC_MESSAGES/*.mo
find_package(Gettext REQUIRED)
file(GLOB _po_dirs LIST_DIRECTORIES true RELATIVE ${CMAKE_CURRENT_SOURCE_DIR}/po ${CMAKE_CURRENT_SOURCE_DIR}/po/*)
foreach(_lang ${_po_dirs})
    if(IS_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}/po/${_lang})
        gettext_process_po_files(${_lang} ALL INSTALL_DESTINATION ${KDE_INSTALL_LOCALEDIR}
            PO_FILES ${CMAKE_CURRENT_SOURCE_DIR}/po/${_lang}/kcm_applekeyboard.po)
    endif()
endforeach()
