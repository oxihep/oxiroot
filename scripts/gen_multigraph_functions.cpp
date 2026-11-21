// A TMultiGraph whose member graphs each carry a function in fFunctions.
// ROOT names TF1 in full for the first one and by a class back-reference for
// the second, which is what the reader must follow across members.
//
//   root -l -b -q scripts/gen_multigraph_functions.cpp
#include "TF1.h"
#include "TFile.h"
#include "TGraph.h"
#include "TList.h"
#include "TMultiGraph.h"

void gen_multigraph_functions() {
    TFile f("fixtures/multigraph_functions.root", "RECREATE", "", 0);

    double x[3] = {1, 2, 3};
    double ya[3] = {2, 4, 6};
    double yb[3] = {1, 4, 9};

    auto* a = new TGraph(3, x, ya);
    a->SetName("a");
    auto* fa = new TF1("lin", "[0]+[1]*x", 0, 4);
    fa->SetParameters(0.0, 2.0);
    a->GetListOfFunctions()->Add(fa);

    auto* b = new TGraph(3, x, yb);
    b->SetName("b");
    auto* fb = new TF1("sq", "[0]*x*x", 0, 4);
    fb->SetParameter(0, 1.0);
    b->GetListOfFunctions()->Add(fb);

    TMultiGraph mg("mg", "two fitted graphs");
    mg.Add(a);
    mg.Add(b);
    mg.Write();
    f.Close();
}
