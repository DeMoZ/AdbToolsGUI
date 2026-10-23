import os
import tempfile
import unittest

from adbtoolkit import macos
from adbtoolkit.core import (
    Device, drop_plan, install_hint, obb_package, parse_devices, parse_ls, remote_join, remote_parent,
)


class ObbPackageTests(unittest.TestCase):
    def test_package_from_name(self):
        cases = {
            "main.123.com.foo.bar.obb": "com.foo.bar",
            "patch.5.com.x_y.z1.obb": "com.x_y.z1",
            r"C:\dl\main.7.org.a.b.obb": "org.a.b",
            "/Users/me/Downloads/main.7.org.a.b.obb": "org.a.b",
            "game.obb": None,
            "main.1.nopkg.obb": None,
        }
        for name, expected in cases.items():
            with self.subTest(name=name):
                self.assertEqual(obb_package(name), expected)


class DropPlanTests(unittest.TestCase):
    def check(self, paths, apks, obbs, aabs, error):
        plan = drop_plan(paths)
        self.assertEqual((len(plan.apks), len(plan.obbs), len(plan.aabs), bool(plan.error)), (apks, obbs, aabs, error))

    def test_cases(self):
        self.check(["a.apk", "b.OBB"], 1, 1, 0, False)
        self.check(["x.aab", "main.1.a.b.obb"], 0, 1, 1, False)
        self.check(["only.obb"], 0, 1, 0, False)
        self.check(["x.aab", "y.apk"], 1, 0, 1, True)
        self.check(["x.aab", "y.aab"], 0, 0, 2, True)
        self.check(["notes.txt"], 0, 0, 0, True)

    def test_folder_is_ignored(self):
        with tempfile.TemporaryDirectory() as d:
            folder = os.path.join(d, "build.apk")
            os.mkdir(folder)
            plan = drop_plan([folder, "a.apk"])
            self.assertEqual(plan.ignored, [folder])
            self.assertEqual(plan.apks, ["a.apk"])


class ParseDevicesTests(unittest.TestCase):
    def test_serial_comes_from_first_column(self):
        text = (
            "* daemon started successfully\n"
            "List of devices attached\n"
            "R9XW30ABCDE            device usb:1-1 product:gta9wifixx model:SM_X115 device:gta9wifi transport_id:1\n"
            "emulator-5554          unauthorized transport_id:2\n"
            "0123456789ABCDEF       no permissions (missing udev rules?); see [http://developer.android.com]\n"
        )
        self.assertEqual(parse_devices(text), [
            Device("R9XW30ABCDE", "device", "SM X115"),
            Device("emulator-5554", "unauthorized", ""),
            Device("0123456789ABCDEF", "no permissions", ""),
        ])


class ParseLsTests(unittest.TestCase):
    def test_toybox_listing(self):
        text = (
            "total 52\n"
            "drwxrws---  2 u0_a123 media_rw 3452 2024-05-01 10:22 .\n"
            "drwxrws---  9 root    media_rw 3452 2024-05-01 10:22 ..\n"
            "drwxrws--x  5 root    media_rw 3452 2024-04-01 09:00 Android\n"
            "-rw-rw----  1 u0_a123 media_rw 1048576 2024-05-02 11:30 My file.mp4\n"
            "lrw-r--r--  1 root    root       21 2024-01-01 00:00 sdcard -> /storage/self/primary\n"
            "ls: ./secret: Permission denied\n"
        )
        entries = parse_ls(text)
        self.assertEqual([e.name for e in entries], ["Android", "sdcard", "My file.mp4"])
        self.assertTrue(entries[0].is_dir)
        self.assertTrue(entries[1].is_link)
        self.assertEqual(entries[2].size, 1048576)

    def test_paths(self):
        self.assertEqual(remote_join("/sdcard/", "DCIM"), "/sdcard/DCIM")
        self.assertEqual(remote_parent("/sdcard/DCIM/"), "/sdcard/")
        self.assertEqual(remote_parent("/sdcard/"), "/")


class InstallHintTests(unittest.TestCase):
    def test_hints(self):
        self.assertIn("подписью", install_hint("Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: ...]"))
        self.assertIn("Auto Blocker", install_hint("INSTALL_FAILED_ABORTED: User rejected permissions"))
        self.assertIsNone(install_hint("Success"))


class MacUsbTests(unittest.TestCase):
    def test_new_format(self):
        items = [{"_name": "USB 3.1 Bus", "_items": [
            {"_name": "Galaxy Tab A9", "USBDeviceKeyVendorID": "0x04e8", "USBDeviceKeyProductID": "0x6860",
             "USBDeviceKeyVendorName": "SAMSUNG", "USBDeviceKeySerialNumber": "R9XW30ABCDE",
             "USBDeviceKeyLinkSpeed": "480 Mb/s"},
            {"_name": "USB3.1 Hub", "USBDeviceKeyVendorID": "0x2109", "_items": [
                {"_name": "2.4G Mouse", "USBDeviceKeyVendorID": "0x1ea7", "USBDeviceKeyProductID": "0x0066"}]},
        ]}]
        devs = macos.parse_usb(items)
        self.assertEqual([d.name for d in devs], ["Galaxy Tab A9", "USB3.1 Hub", "2.4G Mouse"])
        self.assertEqual(devs[0].vendor_id, "04e8")
        self.assertEqual(macos.android_match(devs[0], {"R9XW30ABCDE"}), "adb")
        self.assertEqual(macos.android_match(devs[0], set()), "likely")
        self.assertIsNone(macos.android_match(devs[2], set()))

    def test_old_format(self):
        items = [{"_name": "USB31Bus", "_items": [
            {"_name": "SAMSUNG_Android", "vendor_id": "0x04e8  (Samsung Electronics Co., Ltd.)",
             "product_id": "0x6860", "serial_num": "R9XW30ABCDE", "device_speed": "high_speed"}]}]
        devs = macos.parse_usb(items)
        self.assertEqual(devs[0].vendor, "Samsung Electronics Co., Ltd.")
        self.assertEqual(devs[0].product_id, "6860")
        self.assertEqual(devs[0].serial, "R9XW30ABCDE")


if __name__ == "__main__":
    unittest.main()
