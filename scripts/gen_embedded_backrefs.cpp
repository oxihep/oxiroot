// Dev-only: regenerate fixtures/embedded_backrefs.root.
//
// Objects that hold other objects by pointer, written twice over so that the
// second of each names its classes by a back-reference to the first: a class
// is spelled out once per key and named by its position afterwards.
//
//   "members"  a TList of labelled TH1D and TProfile, TEfficiency (over TH1D
//              and over TH1F), THnSparseD, TF1 and TGraph with a frame
//   "eff_f"    a TEfficiency over TH1F under its own key
//   "eff_2d"   a TEfficiency over TH2D under its own key
//
//   c++ $(root-config --cflags) scripts/gen_embedded_backrefs.cpp \
//       $(root-config --libs) -o /tmp/gen_embedded_backrefs && \
//   (cd <repo-root> && /tmp/gen_embedded_backrefs)

#include <TEfficiency.h>
#include <TF1.h>
#include <TFile.h>
#include <TGraph.h>
#include <TH1D.h>
#include <TH1F.h>
#include <TH2D.h>
#include <THnSparse.h>
#include <TList.h>
#include <TProfile.h>

namespace {

TH1D* labelled(const char* name, double scale) {
    auto* h = new TH1D(name, name, 3, 0, 3);
    h->SetDirectory(nullptr);
    const char* labels[] = {"low", "mid", "high"};
    for (int i = 0; i < 3; ++i) {
        h->GetXaxis()->SetBinLabel(i + 1, labels[i]);
        h->SetBinContent(i + 1, scale * (i + 1));
    }
    return h;
}

template <class H>
TEfficiency* efficiency(const char* name, int passed) {
    H total("total", "total", 4, 0, 4);
    H pass("pass", "pass", 4, 0, 4);
    for (int i = 1; i <= 4; ++i) {
        for (int n = 0; n < 10 * i; ++n) total.Fill(i - 0.5);
        for (int n = 0; n < passed * i; ++n) pass.Fill(i - 0.5);
    }
    auto* e = new TEfficiency(pass, total);
    e->SetName(name);
    e->SetDirectory(nullptr);
    return e;
}

THnSparseD* sparse(const char* name, double weight) {
    int bins[2] = {4, 5};
    double lo[2] = {0, 0};
    double hi[2] = {4, 5};
    auto* s = new THnSparseD(name, name, 2, bins, lo, hi, 8); // small chunks
    double a[2] = {0.5, 1.5};
    double b[2] = {2.5, 3.5};
    s->Fill(a, weight);
    s->Fill(b, 2 * weight);
    return s;
}

TGraph* framed(const char* name, double slope) {
    double x[3] = {1, 2, 3};
    double y[3] = {slope, 2 * slope, 3 * slope};
    auto* g = new TGraph(3, x, y);
    g->SetName(name);
    g->GetHistogram(); // the display frame, a TH1F held by pointer
    return g;
}

} // namespace

int main() {
    TFile f("fixtures/embedded_backrefs.root", "RECREATE");
    f.SetCompressionLevel(0); // uncompressed, so the layout is byte-inspectable

    auto* l = new TList();
    l->SetName("members");
    l->Add(labelled("lab1", 1));
    l->Add(labelled("lab2", 10));
    auto* p = new TProfile("prof", "prof", 3, 0, 3);
    p->SetDirectory(nullptr);
    p->GetXaxis()->SetBinLabel(2, "only");
    p->Fill(1.5, 4.0);
    l->Add(p);
    l->Add(efficiency<TH1D>("eff_d1", 5));
    l->Add(efficiency<TH1D>("eff_d2", 2));
    l->Add(efficiency<TH1F>("eff_f", 3));
    l->Add(sparse("sp1", 1));
    l->Add(sparse("sp2", 3));
    l->Add(new TF1("f1", "[0]*x", 0, 1));
    auto* f2 = new TF1("f2", "[0]+[1]*x", 0, 2);
    f2->SetParameters(1.5, 2.5);
    l->Add(f2);
    l->Add(framed("g1", 1));
    l->Add(framed("g2", 2));
    f.WriteObject(l, "members");

    efficiency<TH1F>("eff_f", 3)->Write("eff_f");

    TH2D total("total2", "total2", 2, 0, 2, 2, 0, 2);
    TH2D pass("pass2", "pass2", 2, 0, 2, 2, 0, 2);
    total.Fill(0.5, 0.5, 4);
    pass.Fill(0.5, 0.5, 1);
    TEfficiency e2(pass, total);
    e2.SetDirectory(nullptr);
    e2.Write("eff_2d");

    f.Close();
    return 0;
}
