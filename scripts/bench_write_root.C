// The ROOT-written files for scripts/bench_uproot.py: the same `events`
// columns as `read_timing write` (pt f64, n i32, jets std::vector<double>),
// written by RDataFrame with ROOT's own basket and cluster sizing, Zstd level 5.
//
//   root -l -b -q 'scripts/bench_write_root.C("DIR", N)'

#include <ROOT/RDataFrame.hxx>

#include <cmath>
#include <string>
#include <vector>

void bench_write_root(const char *dir, unsigned long long n)
{
   auto df = ROOT::RDataFrame(n)
                .Define("pt", [](ULong64_t i) { return std::abs(std::sin(double(i) * 0.37)) * 100.0; },
                        {"rdfentry_"})
                .Define("n", [](ULong64_t i) { return int(i % 17); }, {"rdfentry_"})
                .Define("jets",
                        [](ULong64_t i) {
                           std::vector<double> v;
                           for (ULong64_t j = 0; j < i % 5; ++j)
                              v.push_back(double(i * 5 + j) * 0.01);
                           return v;
                        },
                        {"rdfentry_"});
   ROOT::RDF::RSnapshotOptions opts;
   opts.fCompressionAlgorithm = ROOT::RCompressionSetting::EAlgorithm::kZSTD;
   opts.fCompressionLevel = 5;
   const std::string base(dir);
   df.Snapshot("events", base + "/root_tree.root", {"pt", "n", "jets"}, opts);
   opts.fOutputFormat = ROOT::RDF::ESnapshotOutputFormat::kRNTuple;
   df.Snapshot("events", base + "/root_ntuple.root", {"pt", "n", "jets"}, opts);
}
