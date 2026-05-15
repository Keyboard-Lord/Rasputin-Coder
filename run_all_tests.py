#!/usr/bin/env python3
"""
Comprehensive Test Runner for Rasputin
Runs all Rust tests and reports results
"""

import subprocess
import sys
import time
from dataclasses import dataclass
from typing import List, Tuple

@dataclass
class TestResult:
    name: str
    passed: bool
    output: str
    duration: float

class TestRunner:
    def __init__(self):
        self.results: List[TestResult] = []
        self.passed = 0
        self.failed = 0
        
    def run_cargo_test(self, package: str, test_filter: str = None) -> TestResult:
        """Run cargo test for a specific package"""
        name = f"{package}::{test_filter}" if test_filter else package
        print(f"\n🧪 Running {name}...")
        
        cmd = ["cargo", "test", "-p", package]
        if test_filter:
            cmd.append(test_filter)
        cmd.extend(["--", "--nocapture"])
        
        start = time.time()
        try:
            result = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=120,
                cwd="/Users/mcrae/Desktop/Rasputin-1/Rasputin-Coder"
            )
            duration = time.time() - start
            
            success = result.returncode == 0
            if success:
                self.passed += 1
                print(f"  ✅ PASSED ({duration:.1f}s)")
            else:
                self.failed += 1
                print(f"  ❌ FAILED ({duration:.1f}s)")
                
            return TestResult(
                name=name,
                passed=success,
                output=result.stdout + result.stderr,
                duration=duration
            )
        except subprocess.TimeoutExpired:
            self.failed += 1
            print(f"  ⏱️ TIMEOUT (>120s)")
            return TestResult(
                name=name,
                passed=False,
                output="Test timed out after 120 seconds",
                duration=120.0
            )
        except Exception as e:
            self.failed += 1
            print(f"  💥 ERROR: {e}")
            return TestResult(
                name=name,
                passed=False,
                output=str(e),
                duration=0.0
            )
    
    def run_cargo_check(self, package: str) -> TestResult:
        """Run cargo check for a package"""
        name = f"{package}::check"
        print(f"\n🔍 Checking {package}...")
        
        start = time.time()
        try:
            result = subprocess.run(
                ["cargo", "check", "-p", package],
                capture_output=True,
                text=True,
                timeout=60,
                cwd="/Users/mcrae/Desktop/Rasputin-1/Rasputin-Coder"
            )
            duration = time.time() - start
            
            success = result.returncode == 0
            if success:
                self.passed += 1
                print(f"  ✅ COMPILES ({duration:.1f}s)")
            else:
                self.failed += 1
                print(f"  ❌ COMPILATION FAILED ({duration:.1f}s)")
                
            return TestResult(
                name=name,
                passed=success,
                output=result.stderr if result.stderr else result.stdout,
                duration=duration
            )
        except Exception as e:
            self.failed += 1
            return TestResult(
                name=name,
                passed=False,
                output=str(e),
                duration=0.0
            )
    
    def run_build(self) -> TestResult:
        """Run full release build"""
        print(f"\n🔨 Building release binary...")
        
        start = time.time()
        try:
            result = subprocess.run(
                ["cargo", "build", "-p", "rasputin-tui", "--release"],
                capture_output=True,
                text=True,
                timeout=180,
                cwd="/Users/mcrae/Desktop/Rasputin-1/Rasputin-Coder"
            )
            duration = time.time() - start
            
            success = result.returncode == 0
            if success:
                self.passed += 1
                print(f"  ✅ BUILD SUCCESS ({duration:.1f}s)")
            else:
                self.failed += 1
                print(f"  ❌ BUILD FAILED ({duration:.1f}s)")
                
            return TestResult(
                name="release_build",
                passed=success,
                output=result.stderr if result.stderr else result.stdout,
                duration=duration
            )
        except Exception as e:
            self.failed += 1
            return TestResult(
                name="release_build",
                passed=False,
                output=str(e),
                duration=0.0
            )
    
    def test_host_actions(self) -> List[TestResult]:
        """Test all host actions"""
        print("\n" + "="*60)
        print("HOST ACTIONS TESTS")
        print("="*60)
        
        tests = [
            ("rasputin-tui", "create_project_creates_directory_and_config"),
            ("rasputin-tui", "write_repo_model_config_persists_rasputin_json"),
            ("rasputin-tui", "write_file_updates_real_file"),
            ("rasputin-tui", "read_file_reads_existing_file"),
            ("rasputin-tui", "apply_patch_modifies_file"),
            ("rasputin-tui", "run_command_executes_shell"),
            ("rasputin-tui", "attach_project_succeeds_for_existing_dir"),
            ("rasputin-tui", "batch_operations_test"),
        ]
        
        results = []
        for package, test in tests:
            result = self.run_cargo_test(package, test)
            results.append(result)
        
        return results
    
    def test_doc_generator(self) -> List[TestResult]:
        """Test document generator"""
        print("\n" + "="*60)
        print("DOCUMENT GENERATOR TESTS")
        print("="*60)
        
        tests = [
            ("rasputin-tui", "test_canonical_docs_count"),
            ("rasputin-tui", "test_doc_numbering"),
            ("rasputin-tui", "test_progress_calculation"),
        ]
        
        results = []
        for package, test in tests:
            result = self.run_cargo_test(package, test)
            results.append(result)
        
        return results
    
    def test_commands(self) -> List[TestResult]:
        """Test command parsing"""
        print("\n" + "="*60)
        print("COMMAND PARSING TESTS")
        print("="*60)
        
        tests = [
            ("rasputin-tui", "exact_goal_subcommands_win_before_goal_text"),
            ("rasputin-tui", "goal_text_becomes_goal_command"),
        ]
        
        results = []
        for package, test in tests:
            result = self.run_cargo_test(package, test)
            results.append(result)
        
        return results
    
    def test_forge_runtime(self) -> List[TestResult]:
        """Test forge runtime"""
        print("\n" + "="*60)
        print("FORGE RUNTIME TESTS")
        print("="*60)
        
        # Check compilation
        check_result = self.run_cargo_check("forge_bootstrap")
        
        # Run tests
        result = self.run_cargo_test("forge_bootstrap")
        
        return [check_result, result]
    
    def test_batch_tools(self) -> List[TestResult]:
        """Test batch processing tools"""
        print("\n" + "="*60)
        print("BATCH TOOLS TESTS")
        print("="*60)
        
        # Check batch_tools module compiles
        check_result = self.run_cargo_check("forge_bootstrap")
        
        return [check_result]
    
    def run_all(self):
        """Run all tests"""
        print("\n" + "="*60)
        print("RASPUTIN COMPREHENSIVE TEST SUITE")
        print("="*60)
        print(f"Started at: {time.strftime('%Y-%m-%d %H:%M:%S')}")
        
        all_results = []
        
        # Build first
        build_result = self.run_build()
        all_results.append(build_result)
        
        if not build_result.passed:
            print("\n❌ Build failed - skipping unit tests")
        else:
            # Run all test suites
            all_results.extend(self.test_host_actions())
            all_results.extend(self.test_doc_generator())
            all_results.extend(self.test_commands())
            all_results.extend(self.test_forge_runtime())
            all_results.extend(self.test_batch_tools())
        
        # Print summary
        self.print_summary(all_results)
        
        return self.failed == 0
    
    def print_summary(self, results: List[TestResult]):
        """Print test summary"""
        print("\n" + "="*60)
        print("TEST SUMMARY")
        print("="*60)
        
        total = len(results)
        passed = sum(1 for r in results if r.passed)
        failed = total - passed
        total_time = sum(r.duration for r in results)
        
        print(f"\nTotal:  {total}")
        print(f"Passed: {passed} ✅")
        print(f"Failed: {failed} ❌")
        print(f"Time:   {total_time:.1f}s")
        
        if failed > 0:
            print("\n❌ FAILED TESTS:")
            for result in results:
                if not result.passed:
                    print(f"  - {result.name}")
                    # Print first line of error
                    first_error = result.output.split('\n')[0][:80]
                    print(f"    {first_error}...")
        
        print("\n" + "="*60)
        if failed == 0:
            print("🎉 ALL TESTS PASSED! 🎉")
        else:
            print(f"⚠️  {failed} TEST(S) FAILED")
        print("="*60 + "\n")

if __name__ == "__main__":
    runner = TestRunner()
    success = runner.run_all()
    sys.exit(0 if success else 1)
