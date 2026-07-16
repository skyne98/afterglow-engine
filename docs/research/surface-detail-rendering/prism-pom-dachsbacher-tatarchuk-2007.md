     Prism Parallax Occlusion Mapping with Accurate Silhouette
                            Generation
                                Carsten Dachsbacher, Natalya Tatarchuk



      To cite this version:
     Carsten Dachsbacher, Natalya Tatarchuk. Prism Parallax Occlusion Mapping with Accurate Silhouette Gen-
     eration. 2007. ⟨inria-00606806⟩




                                       HAL Id: inria-00606806
                         https://inria.hal.science/inria-00606806v1
                                            Submitted on 18 Jul 2011




    HAL is a multi-disciplinary open access archive             L’archive ouverte pluridisciplinaire HAL, est des-
for the deposit and dissemination of scientific re-        tinée au dépôt et à la diffusion de documents scien-
search documents, whether they are published or not.       tifiques de niveau recherche, publiés ou non, émanant
The documents may come from teaching and research          des établissements d’enseignement et de recherche
institutions in France or abroad, or from public or pri-   français ou étrangers, des laboratoires publics ou
vate research centers.                                     privés.


                                              HAL Authorization
     Prism Parallax Occlusion Mapping with Accurate Silhouette Generation
    Carsten Dachsbacher / REVES/INRIA Sophia-Antipolis ∗                 Natalya Tatarchuk / 3D Application Research Group/AMD †




Per-pixel displacement mapping algorithms such as [Policarpo
et al. 2005; Tatarchuk 2006] became very popular recently as they
can take advantage of the parallel nature of programmable GPU
pipelines and render detailed surfaces at highly interactive rates.
These approaches exhibit pleasing visual quality and render motion
parallax effects, however, most of them suffer from lack of correct
silhouettes. We perform ray-surface intersection in a volume given
by prisms extruded from the input mesh triangles in the direction
of the normal. The displaced surface is embedded in the volume
of these prisms, bounded by a top and a bottom triangle and three
bilinear patches (slabs). [Hirche et al. 2004] propose to triangu-
late the slabs and split the prisms into three tetrahedra. A consis-
tent triangulation of adjacent prisms ensures that no gaps between
tetrahedra exist and no tetrahedra overlap. Ray marching through
tetrahedra is then straightforward as texture gradients (for marching
along the ray) can be computed per tetrahedron.

1     Prism Parallax Occlusion Mapping
In contrast to previous work, we propose to directly operate on the
prisms: The key observation on which we base our method is that
we achieve visually pleasing results if we compute correct texture
coordinates on the prism surface and rely on a constant texture gra-
dient thereof for each intersecting view ray. To this end, we com-        Figure 1: Left: to approximate the horizon we sample distant eleva-
pute intersections of view rays and prisms and the corresponding          tion features at coarser resolution. Right (top to bottom): ambient
texture coordinates in pixel shaders directly. Slabs are rasterized as    occlusion (4 directions), hard and soft shadows and final rendering
two triangles; the split diagonal has to be chosen such that the re-      with our horizon approximation.
sulting triangular surface is pointing outwards and true intersections
are computed per-pixel [Ramsey et al. 2004]. To avoid superfluous         3     Results and Discussion
intersection tests, we use early rejection tests during geometry pro-
cessing.                                                                  We implemented our ray-casting method for DirectXr9 class hard-
                                                                          ware by preprocessing the input meshes to extrude the triangles, and
                                                                          then rendering them on the GPU achieving frame rates of 79 fps for
2     Shadows and Ambient Occlusion                                       a 500 triangle sphere model (top figure) and 59 fps for a 7K triangle
                                                                          cylinder model on a ATIrRadeonrX1950 graphics card.
We use a fast approximation to the horizon angle on height fields
to compute ambient occlusion and shadows for local surface fea-           Our shadow by horizon approximation technique runs at compara-
tures at render time: Close elevations have greater impact on the         ble speed as the POM shadowing technique. Computing per-pixel
horizon angle, whereas small scale detail at greater distance can be      ambient occlusion causes a 35% drop in performance for 4 static
neglected. For the height map, we generate mip-map levels, but in-        directions, 60% for 8 randomized directions.
stead of averaging pixels when going to the next level, we store their
maximum value. For our estimation we then use k height samples            References
                                                                          H IRCHE , J., E HLERT, A., G UTHE , S., AND D OGGETT, M. 2004. Hardware ac-
along the view direction, with a distance of 2k , k ≥ 0 texels to the         celerated per-pixel displacement mapping. In GI ’04: Proceedings of the 2004
point in question to estimate the horizon slope. To omit the small            conference on Graphics interface.
scale detail, we use the mip-map level k respectively.                    P OLICARPO , F., O LIVEIRA , M. M., AND C OMBA , J. L. D. 2005. Real-time relief
                                                                             mapping on arbitrary polygonal surfaces. In Symposium on Interactive 3D Graph-
We also compute per-pixel ambient occlusion by estimating the the            ics and Games 2005, ACM Press, 155–162.
horizon approximation for several directions (4 to 8 in our tests)
                                                                          R AMSEY, S. D., P OTTER , K., AND H ANSEN , C. 2004. Ray bilinear patch intersec-
splitting the hemisphere above the query point into equal sized sec-         tions. Journal of Graphics Tools 9, 3, 41–47.
tors; per-pixel rotated directions conceal regular structures.
                                                                          TATARCHUK , N. 2006. Dynamic parallax occlusion mapping with approximate soft
    ∗ e-mail: Carsten.Dachsbacher@sophia.inria.fr                            shadows. In Symposium on Interactive 3D Graphics and Games 2006, ACM Press,
    † e-mail:natalya.tatarchuk@amd.com                                       63–69.
