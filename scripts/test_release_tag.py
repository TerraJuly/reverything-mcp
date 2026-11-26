"""Pure release-version tests using synthetic strings, with no file searches."""

import unittest

from check_release_tag import validate_tag


class ReleaseTagTests(unittest.TestCase):
    def test_stable_version(self):
        self.assertEqual(validate_tag("v0.1.0", "0.1.0"), {
            "version": "0.1.0", "prerelease": "false",
            "bundle_name": "reverything-mcp-0.1.0-windows-x64",
        })

    def test_prerelease_and_build_metadata(self):
        self.assertEqual(validate_tag("v0.2.0-rc.1", "0.2.0-rc.1")["prerelease"], "true")
        self.assertEqual(validate_tag("v1.0.0+build.01", "1.0.0+build.01")["prerelease"], "false")
        self.assertEqual(validate_tag("v1.0.0-alpha+build", "1.0.0-alpha+build")["prerelease"], "true")

    def test_mismatching_package_version(self):
        with self.assertRaises(ValueError):
            validate_tag("v0.2.0", "0.1.0")

    def test_malformed_versions_and_injection_strings(self):
        for tag in [
            "0.1.0", "v1", "v1.2", "v01.2.3", "v1.2.03", "v1.2.3-01",
            "v1.2.3-", "v1.2.3-alpha..1", "v1.2.3+", "v0.1.0\n",
            "v0.1.0;echo unsafe", "v0.1.0$(echo unsafe)", "v0.1.0\nprerelease=false",
        ]:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                validate_tag(tag, tag.removeprefix("v"))


class PrivateTestResult(unittest.TextTestResult):
    def _exc_info_to_string(self, err, test):
        return f"{err[0].__name__}: traceback paths and diagnostic details omitted for privacy"


if __name__ == "__main__":
    unittest.main(testRunner=unittest.TextTestRunner(resultclass=PrivateTestResult))
